//! A small dynamic value type for mpv properties (`mpv_node` in, owned data out).

/// Owned counterpart of `mpv_node`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    /// Key/value pairs in the order mpv reported them.
    Map(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Value {
        match self {
            Value::Map(entries) => entries
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::None),
            _ => Value::None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(v) => Some(*v),
            Value::Int(v) => Some(*v as f64),
            Value::Str(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(v) => *v,
            Value::Int(v) => *v != 0,
            Value::Str(s) => matches!(s.as_str(), "yes" | "true" | "on"),
            _ => false,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> &[Value] {
        match self {
            Value::Array(items) => items.as_slice(),
            _ => &[],
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Value::None)
    }

    /// `number | string | bool` shorthand used by the handful of places that need to push
    /// a scalar straight back into mpv.
    pub fn number(value: f64) -> Value {
        Value::Float(value)
    }

    /// Deep-copy a libmpv `mpv_node` into owned data.
    ///
    /// Strings and nested lists/maps are copied, so the result may outlive the node
    /// (mpv frees event nodes after the next `mpv_wait_event`).
    ///
    /// # Safety
    /// `node` must point at a fully initialized `mpv_node`.
    pub unsafe fn from_mpv_node(node: &crate::ffi::mpv_node) -> Value {
        use crate::ffi::{
            MPV_FORMAT_DOUBLE, MPV_FORMAT_FLAG, MPV_FORMAT_INT64, MPV_FORMAT_NODE_ARRAY,
            MPV_FORMAT_NODE_MAP, MPV_FORMAT_NONE, MPV_FORMAT_STRING,
        };
        match node.format {
            MPV_FORMAT_NONE => Value::None,
            MPV_FORMAT_STRING | crate::ffi::MPV_FORMAT_OSD_STRING => {
                let ptr = unsafe { node.u.string };
                if ptr.is_null() {
                    Value::None
                } else {
                    Value::Str(
                        unsafe { std::ffi::CStr::from_ptr(ptr) }
                            .to_string_lossy()
                            .into_owned(),
                    )
                }
            }
            MPV_FORMAT_FLAG => Value::Bool(unsafe { node.u.flag } != 0),
            MPV_FORMAT_INT64 => Value::Int(unsafe { node.u.int64 }),
            MPV_FORMAT_DOUBLE => Value::Float(unsafe { node.u.double_ }),
            MPV_FORMAT_NODE_ARRAY | MPV_FORMAT_NODE_MAP => {
                let list = unsafe { node.u.list };
                if list.is_null() {
                    return Value::None;
                }
                let list = unsafe { &*list };
                let num = list.num.max(0) as usize;
                let mut items = Vec::with_capacity(num);
                for i in 0..num {
                    let item = unsafe { &*list.values.add(i) };
                    if node.format == MPV_FORMAT_NODE_MAP {
                        let key = unsafe { *list.keys.add(i) };
                        let key = if key.is_null() {
                            String::new()
                        } else {
                            unsafe { std::ffi::CStr::from_ptr(key) }
                                .to_string_lossy()
                                .into_owned()
                        };
                        items.push((key, unsafe { Value::from_mpv_node(item) }));
                    } else {
                        items.push((String::new(), unsafe { Value::from_mpv_node(item) }));
                    }
                }
                if node.format == MPV_FORMAT_NODE_MAP {
                    Value::Map(items.into_iter().map(|(k, v)| (k, v)).collect())
                } else {
                    Value::Array(items.into_iter().map(|(_, v)| v).collect())
                }
            }
            _ => Value::None,
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(v as i64)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Str(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_lookup() {
        let value = Value::Map(vec![
            ("id".into(), Value::Int(2)),
            ("type".into(), Value::Str("video".into())),
        ]);
        assert_eq!(value.get("id").as_i64(), Some(2));
        assert_eq!(value.get("type").as_str(), Some("video"));
        assert!(value.get("missing").is_none());
    }

    #[test]
    fn numeric_coercion() {
        assert_eq!(Value::Str("12.5".into()).as_f64(), Some(12.5));
        assert_eq!(Value::Float(3.0).as_i64(), Some(3));
        assert!(Value::Str("yes".into()).as_bool());
        assert!(!Value::None.as_bool());
    }
}
