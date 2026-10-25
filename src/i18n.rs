//! UI string translation with stable keys.
//!
//! Call sites keep the English string as the key (`tr("Open media")`), so adding a
//! string never needs a new identifier and a missing entry degrades to English
//! instead of panicking. The active language is a process global set at startup and
//! whenever preferences change.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lang {
    En,
    Fa,
}

impl Lang {
    pub fn from_str(value: &str) -> Self {
        match value {
            "fa" => Self::Fa,
            _ => Self::En,
        }
    }
}

static ACTIVE_LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(lang: Lang) {
    ACTIVE_LANG.store(u8::from(lang == Lang::Fa), Ordering::Release);
}

pub fn lang() -> Lang {
    match ACTIVE_LANG.load(Ordering::Acquire) {
        1 => Lang::Fa,
        _ => Lang::En,
    }
}

/// `true` when the active UI language is right-to-left (Persian).
pub fn rtl() -> bool {
    lang() == Lang::Fa
}

/// Translate the English `key` into the active UI language.
pub fn tr(key: &str) -> &str {
    match lang() {
        Lang::En => key,
        Lang::Fa => fa(key),
    }
}

/// Persian table: every entry maps one stable English key. Missing keys fall back to
/// English via the `match` default, so a forgotten entry can never break the build.
fn fa(key: &str) -> &str {
    match key {
        // -- shell / dialogs ------------------------------------------------------------------------
        "Preferences" => "تنظیمات",
        "Playlist" => "پلی‌لیست",
        // -- control bar ---------------------------------------------------------------------------
        "Pause" => "توقف موقت",
        "Play" => "پخش",
        "Control panel" => "پنل کنترل",
        "Mute" => "بی‌صدا",
        "Fullscreen" => "تمام‌صفحه",
        "Click to switch between elapsed and remaining time" => {
            "برای تغییر بین زمان گذشته و باقی‌مانده کلیک کنید"
        }
        // -- playlist panel ------------------------------------------------------------------------
        "UP NEXT" => "بعدی",
        "Filter" => "فیلتر",
        "Add" => "افزودن",
        "More" => "بیشتر",
        "Sort" => "مرتب‌سازی",
        "Repeat" => "تکرار",
        "Shuffle" => "پخش تصادفی",
        "Loading…" => "در حال بارگذاری…",
        "Duration unavailable" => "مدت‌زمان نامشخص",
        "Network media" => "رسانه شبکه",
        "Local media" => "رسانه محلی",
        "Playback failed" => "پخش ناموفق",
        "Unavailable" => "در دسترس نیست",
        "URL" => "نشانی",
        "FILE" => "فایل",
        "ERROR" => "خطا",
        "MISSING" => "یافت نشد",
        // -- OSD -----------------------------------------------------------------------------------
        "Chapter" => "فصل",
        "No chapters" => "فصلی وجود ندارد",
        "Bookmarks" => "نشانک‌ها",
        "Bookmark saved" => "نشانک ذخیره شد",
        "A-B repeat: off" => "تکرار A-B: خاموش",
        "Volume" => "صدا",
        "Could not open media" => "باز کردن رسانه ممکن نشد",
        // -- dialog chrome -------------------------------------------------------------------------
        "Close" => "بستن",
        "OK" => "تأیید",
        "Cancel" => "لغو",
        "Apply" => "اعمال",
        "General" => "عمومی",
        "Playback" => "پخش",
        "Subtitles" => "زیرنویس",
        "Audio" => "صدا",
        "Video" => "ویدیو",
        // -- preferences ---------------------------------------------------------------------------
        "Resize the window to the video when a file opens" => {
            "باز شدن پنجره به اندازه‌ی ویدیو هنگام باز کردن فایل"
        }
        "Largest window size (of the screen)" => "بزرگ‌ترین اندازه‌ی پنجره (نسبت به صفحه)",
        "Remember the playback position of each file" => "به خاطر سپردن موقعیت پخش هر فایل",
        "Always on top" => "همیشه رو",
        "Never" => "هرگز",
        "Always" => "همیشه",
        "While playing" => "هنگام پخش",
        "Show the remaining time instead of the total" => "نمایش زمان باقی‌مانده به‌جای کل",
        "Tone-map HDR video for standard displays" => "تبدیل HDR برای نمایشگرهای معمولی",
        "HDR tone mapping" => "نگاشت HDR",
        "Fullscreen monitor" => "مانیتور تمام‌صفحه",
        "Screenshot folder" => "پوشه‌ی عکس‌صفحه",
        "Browse…" => "انتخاب…",
        "Seek step: Left / Right" => "گام پرش: Left / Right",
        "Seek step: Ctrl + Left / Right" => "گام پرش: Ctrl + Left / Right",
        "Seek step: Shift + Left / Right" => "گام پرش: Shift + Left / Right",
        "Seek step: Ctrl + Alt + Left / Right" => "گام پرش: Ctrl + Alt + Left / Right",
        "Volume step" => "گام صدا",
        "Use hardware video decoding when available" => "استفاده از رمزگشای سخت‌افزاری در صورت وجود",
        "Font" => "فونت",
        "Use the default font" => "استفاده از فونت پیش‌فرض",
        "Bold" => "درشت",
        "Text encoding for non-Unicode files" => "رمزنگاری متن فایل‌های غیر یونیکد",
        "Load subtitles automatically" => "بارگذاری خودکار زیرنویس",
        "Same name as the video" => "هم‌نام ویدیو",
        "Names containing the video name" => "نام‌های شامل نام ویدیو",
        "Every subtitle in the folder" => "همه‌ی زیرنویس‌های پوشه",
        "Do not load automatically" => "بدون بارگذاری خودکار",
        "Preferred subtitle languages" => "زبان‌های ترجیحی زیرنویس",
        "Preferred audio languages" => "زبان‌های ترجیحی صدا",
        "Theme" => "پوسته",
        "Dark" => "تیره",
        "Light" => "روشن",
        "Accent color" => "رنگ تأکیدی",
        "Language" => "زبان",
        "English" => "English",
        "Persian" => "فارسی",
        "Gold" => "طلایی",
        "Blue" => "آبی",
        "Green" => "سبز",
        "Rose" => "رز",
        "Graphite" => "گرافیتی",
        "Media files" => "فایل‌های رسانه",
        "Subtitle files" => "فایل‌های زیرنویس",
        "Audio files" => "فایل‌های صوتی",
        "Frame copied to clipboard" => "فریم در کلیپ‌بورد کپی شد",
        "Could not capture the current frame" => "گرفتن فریم فعلی ممکن نشد",
        "Could not read the captured frame" => "خواندن فریم گرفته‌شده ممکن نشد",
        "Open a video first, then add the subtitle" => {
            "ابتدا یک ویدیو باز کنید، سپس زیرنویس اضافه کنید"
        }
        "No playable files found" => "فایل قابل پخشی پیدا نشد",
        "The clipboard does not contain a file or URL" => "کلیپ‌بورد فایل یا نشانی ندارد",
        "Image adjustments reset" => "تنظیمات تصویر بازنشانی شد",
        "Pan & Scan reset" => "پن و اسکن بازنشانی شد",
        "Subtitle size and position reset" => "اندازه و موقعیت زیرنویس بازنشانی شد",
        "LOAD" => "بارگذاری",
        // -- control panel -------------------------------------------------------------------------
        "Enable equalizer" => "فعال‌سازی اکولایزر",
        "Normalize volume" => "نرمال‌سازی صدا",
        "Reset" => "بازنشانی",
        "Normal Speed" => "سرعت عادی",
        "Set A" => "تعیین A",
        "Set B" => "تعیین B",
        "Clear" => "پاک کردن",
        "Show subtitles" => "نمایش زیرنویس",
        "Size" => "اندازه",
        "Vertical position" => "موقعیت عمودی",
        // -- go to / url dialogs -------------------------------------------------------------------
        "Go to" => "پرش به",
        "Open a video" => "یک ویدیو باز کنید",
        "HH:MM:SS or 01:30" => "HH:MM:SS یا 01:30",
        "Please enter a valid time" => "لطفاً زمان معتبر وارد کنید",
        "Please enter a number" => "لطفاً عدد وارد کنید",
        "Play URL" => "پخش نشانی اینترنتی",
        "Please enter a valid URL" => "لطفاً نشانی معتبر وارد کنید",
        "Drop to load subtitle" => "رها کنید تا زیرنویس بارگذاری شود",
        "Drop to add to playlist" => "رها کنید تا به پلی‌لیست اضافه شود",
        "Drop to play" => "رها کنید تا پخش شود",
        "Playlist is empty" => "پلی‌لیست خالی است",
        "No matching items" => "موردی پیدا نشد",
        "Add files, a folder, or a URL to start playing" => {
            "برای شروع، فایل، پوشه یا نشانی اضافه کنید"
        }
        "Try a different filter" => "فیلتر دیگری امتحان کنید",
        "Sort By" => "مرتب‌سازی بر اساس",
        "Resume playback" => "ادامه‌ی پخش",
        "You previously watched this file up to" => "این فایل را قبلاً تا اینجا دیده‌اید:",
        "Resume from" => "ادامه از",
        "Start from beginning" => "از ابتدا شروع کن",
        "Time" => "زمان",
        "Percent" => "درصد",
        "Drop a file here · ⌘O · or press play" => {
            "فایل را اینجا رها کنید · ⌘O · یا پخش را بزنید"
        }
        "Seek back" => "پرش به عقب",
        "Seek forward" => "پرش به جلو",
        "Settings" => "تنظیمات",
        "Resume where you left off" => "ادامه از جای رهاشده",
        "Hardware decoding" => "رمزگشایی سخت‌افزاری",
        "All settings are in Preferences" => "بقیه تنظیمات در Preferences",
        "Continue watching?" => "ادامه تماشا؟",
        "Copy" => "کپی",
        // -- command palette -----------------------------------------------------------------------
        "Search commands…" => "جستجوی فرمان‌ها…",
        "No matching commands" => "فرمانی پیدا نشد",
        "↑↓ navigate · ↵ run · esc close" => "↑↓ انتخاب · ↵ اجرا · esc بستن",
        // -- misc ----------------------------------------------------------------------------------
        "On" => "روشن",
        "Off" => "خاموش",
        "Open" => "باز کردن",
        "Stop" => "توقف",
        "Play / Pause" => "پخش / توقف",
        "About" => "درباره",
        "Back" => "عقب",
        "Forward" => "جلو",
        "sec" => "ثانیه",
        "Speed" => "سرعت",
        "Normal" => "عادی",
        "Faster" => "تندتر",
        "Slower" => "کندتر",
        "(none)" => "(هیچ)",
        "Remove" => "حذف",
        "Active" => "فعال",
        "none (software)" => "هیچ (نرم‌افزاری)",
        "Preferences..." => "تنظیمات...",
        // -- about ---------------------------------------------------------------------------------
        "A fast, native media player for macOS — built with Rust, egui and libmpv." => {
            "یک پخش‌کنندهٔ سریع و بومی برای macOS — ساخته‌شده با Rust و egui و libmpv."
        }
        "Engine" => "موتور پخش",
        "Tips" => "نکته‌ها",
        "Press ⌘K for the command palette." => "برای پالت فرمان‌ها ⌘K را بزنید.",
        // -- OSD messages ---------------------------------------------------------------------------
        "Last file" => "فایل آخر",
        "First file" => "فایل اول",
        "Next chapter" => "فصل بعدی",
        "Previous chapter" => "فصل قبلی",
        "Next Chapter" => "فصل بعدی",
        "Previous Chapter" => "فصل قبلی",
        "Next frame" => "فریم بعدی",
        "Previous frame" => "فریم قبلی",
        "Buffering" => "در حال بافر",
        "A-B repeat" => "تکرار A-B",
        "start point" => "نقطهٔ شروع",
        "end point" => "نقطهٔ پایان",
        "Repeat: off" => "تکرار: خاموش",
        "Repeat: current file" => "تکرار: فایل فعلی",
        "Repeat: playlist" => "تکرار: پلی‌لیست",
        "Current File" => "فایل فعلی",
        "Shuffle: on" => "پخش تصادفی: روشن",
        "Shuffle: off" => "پخش تصادفی: خاموش",
        "Remember playback position: on" => "به خاطر سپردن موقعیت پخش: روشن",
        "Remember playback position: off" => "به خاطر سپردن موقعیت پخش: خاموش",
        "Remember Playback Position" => "به خاطر سپردن موقعیت پخش",
        "Resumed at" => "ادامه از",
        "Started from the beginning" => "از ابتدا شروع شد",
        "Subtitles: show" => "زیرنویس‌ها: نمایش",
        "Subtitles: hide" => "زیرنویس‌ها: پنهان",
        "Subtitle" => "زیرنویس",
        "No subtitle tracks" => "زیرنویسی وجود ندارد",
        "No audio tracks" => "قطعهٔ صوتی وجود ندارد",
        "No video tracks" => "قطعهٔ ویدیو وجود ندارد",
        "Subtitle sync" => "همگام‌سازی زیرنویس",
        "Subtitle sync (sec)" => "همگام‌سازی زیرنویس (ثانیه)",
        "Subtitle size" => "اندازهٔ زیرنویس",
        "Subtitle position" => "موقعیت زیرنویس",
        "Subtitle encoding" => "رمزنگاری زیرنویس",
        "Audio sync" => "همگام‌سازی صدا",
        "Audio sync (sec):" => "همگام‌سازی صدا (ثانیه):",
        "Keep aspect ratio" => "حفظ نسبت تصویر",
        "Fill window (crop)" => "پر کردن پنجره (برش)",
        "Stretch to window" => "کشیدن به اندازهٔ پنجره",
        "Original size" => "اندازهٔ اصلی",
        "Keep Aspect Ratio" => "حفظ نسبت تصویر",
        "Fill Window (Crop)" => "پر کردن پنجره (برش)",
        "Stretch to Window" => "کشیدن به اندازهٔ پنجره",
        "Original Size" => "اندازهٔ اصلی",
        "Zoom" => "بزرگ‌نمایی",
        "Zoom In" => "بزرگ‌نمایی",
        "Zoom Out" => "کوچک‌نمایی",
        "Rotation" => "چرخش",
        "Rotate 90° Clockwise" => "چرخش ۹۰ درجه ساعتگرد",
        "Flip Horizontally" => "آینهٔ افقی",
        "Flip Vertically" => "آینهٔ عمودی",
        "Move Left" => "انتقال به چپ",
        "Move Right" => "انتقال به راست",
        "Move Up" => "انتقال به بالا",
        "Move Down" => "انتقال به پایین",
        "Brightness" => "روشنایی",
        "Contrast" => "کنتراست",
        "Saturation" => "اشباع رنگ",
        "Hue" => "فام",
        "Gamma" => "گاما",
        "Deinterlace" => "حذف درهم‌بافتگی",
        "Use Hardware Decoding" => "رمزگشایی سخت‌افزاری",
        "Software Decoding" => "رمزگشایی نرم‌افزاری",
        "Window size" => "اندازهٔ پنجره",
        "limited by screen" => "محدود به صفحه",
        "Window size is not available in full screen" => {
            "اندازهٔ پنجره در حالت تمام‌صفحه قابل تغییر نیست"
        }
        "Fit Window to Video on Open" => "تنظیم پنجره به اندازهٔ ویدیو هنگام باز کردن",
        "On top: never" => "همیشه رو: هرگز",
        "On top: always" => "همیشه رو: همیشه",
        "On top: while playing" => "همیشه رو: هنگام پخش",
        "On Top" => "همیشه رو",
        "While Playing" => "هنگام پخش",
        "Jump to Beginning" => "پرش به ابتدا",
        "Go to Time..." => "پرش به زمان...",
        "Previous Subtitle Line" => "خط زیرنویس قبلی",
        "Next Subtitle Line" => "خط زیرنویس بعدی",
        "Set Start Point (A)" => "تعیین نقطهٔ شروع (A)",
        "Set End Point (B)" => "تعیین نقطهٔ پایان (B)",
        "Clear A-B Repeat" => "پاک کردن تکرار A-B",
        "Equalizer..." => "اکولایزر...",
        "Control Panel..." => "پنل کنترل...",
        "Save Current Frame" => "ذخیرهٔ فریم فعلی",
        "Save Current Frame (With Subtitles)" => "ذخیرهٔ فریم فعلی (با زیرنویس)",
        "Copy Current Frame" => "کپی فریم فعلی",
        "Copy Current Frame (With Subtitles)" => "کپی فریم فعلی (با زیرنویس)",
        "Open Screenshot Folder" => "باز کردن پوشهٔ عکس‌صفحه",
        "Open a video first" => "ابتدا یک ویدیو باز کنید",
        "Added" => "اضافه شد",
        "file to the playlist" => "فایل به پلی‌لیست",
        "files to the playlist" => "فایل به پلی‌لیست",
        "Playlist change undone" => "تغییر پلی‌لیست برگردانده شد",
        "Playlist change redone" => "تغییر پلی‌لیست دوباره انجام شد",
        "Path copied to the clipboard" => "مسیر در کلیپ‌بورد کپی شد",
        "Could not read the clipboard" => "خواندن کلیپ‌بورد ممکن نشد",
        "Could not copy frame" => "کپی فریم ممکن نشد",
        "Cannot create folder" => "ساخت پوشه ممکن نشد",
        "Could not save settings" => "ذخیرهٔ تنظیمات ممکن نشد",
        "Saved" => "ذخیره شد",
        "Right-click the video for the full menu." => {
            "برای دیدن منوی کامل، روی ویدیو راست‌کلیک کنید."
        }
        "Settings: ~/Library/Application Support/MacPlayer/settings.json" => {
            "تنظیمات: ~/Library/Application Support/MacPlayer/settings.json"
        }
        // -- playlist menu ---------------------------------------------------------------------------
        "Add Files..." => "افزودن فایل‌ها...",
        "Add Folder..." => "افزودن پوشه...",
        "Add URL..." => "افزودن نشانی...",
        "Remove Selected" => "حذف انتخاب‌شده‌ها",
        "Remove Missing Files" => "حذف فایل‌های گم‌شده",
        "Remove Duplicates" => "حذف تکراری‌ها",
        "Clear Playlist" => "پاک کردن پلی‌لیست",
        "By Name" => "بر اساس نام",
        "By Path" => "بر اساس مسیر",
        "By Duration" => "بر اساس مدت",
        "Reverse" => "معکوس",
        "Randomize" => "ترتیب تصادفی",
        "Name" => "نام",
        "Path" => "مسیر",
        "Duration" => "مدت",
        "Reverse Order" => "ترتیب معکوس",
        "Copy Path" => "کپی مسیر",
        "Show in File Manager" => "نمایش در فایل‌منیجر",
        // -- menu entries ----------------------------------------------------------------------------
        "Misc" => "متفرقه",
        "Window Size" => "اندازهٔ پنجره",
        "A-B Repeat" => "تکرار A-B",
        "Next File" => "فایل بعدی",
        "Previous File" => "فایل قبلی",
        "Next Frame" => "فریم بعدی",
        "Previous Frame" => "فریم قبلی",
        "Show Subtitles" => "نمایش زیرنویس",
        "0.5 sec Faster" => "نیم ثانیه جلوتر",
        "0.5 sec Slower" => "نیم ثانیه عقب‌تر",
        "Larger" => "بزرگ‌تر",
        "Smaller" => "کوچک‌تر",
        "Audio Sync" => "همگام‌سازی صدا",
        "0.05 sec Faster" => "پنج‌صدم ثانیه جلوتر",
        "0.05 sec Slower" => "پنج‌صدم ثانیه عقب‌تر",
        "Reset Sync" => "بازنشانی همگام‌سازی",
        "Volume Up" => "بلندتر",
        "Volume Down" => "آرام‌تر",
        "Normalize Volume" => "نرمال‌سازی صدا",
        "Subtitle Settings..." => "تنظیمات زیرنویس...",
        // -- dialogs --------------------------------------------------------------------------------
        "Audio track" => "قطعهٔ صوتی",
        "Audio volume (%)" => "بلندی صدا (٪)",
        "Preset..." => "پیش‌تنظیم...",
        "Enter URL:" => "نشانی را وارد کنید:",
        "Loop interval (seconds, 0 = no loop):" => "فاصلهٔ تکرار (ثانیه، ۰ = بدون تکرار):",
        "Loop interval must be a whole number from 0 to 86400" => {
            "فاصلهٔ تکرار باید عددی صحیح بین ۰ تا ۸۶۴۰۰ باشد"
        }
        "Track" => "قطعه",
        "Auto" => "خودکار",
        "Auto (system default)" => "خودکار (پیش‌فرض سیستم)",
        "Select Audio Stream" => "انتخاب جریان صدا",
        "Secondary Subtitle" => "زیرنویس دوم",
        "Subtitle Sync" => "همگام‌سازی زیرنویس",
        "Size and Position" => "اندازه و موقعیت",
        "Encoding" => "رمزنگاری",
        "Aspect Ratio" => "نسبت تصویر",
        "Frame Size" => "اندازه‌ی قاب",
        "Pan & Scan" => "پن و اسکن",
        "Rotate and Flip" => "چرخش و آینه",
        "Image Adjustments" => "تنظیم تصویر",
        "Hardware Acceleration" => "شتاب‌دهی سخت‌افزاری",
        "Video Track" => "قطعه ویدیو",
        "Output Device" => "دستگاه خروجی",
        "Jump" => "پرش",
        "Frame Step" => "گام فریم",
        "Chapters" => "فصل‌ها",
        "Recent Files" => "فایل‌های اخیر",
        "Clear Recent Files" => "پاک کردن فایل‌های اخیر",
        "Add Bookmark" => "افزودن نشانک",
        "Close Playback" => "بستن پخش",
        "Open File(s)..." => "باز کردن فایل‌ها...",
        "Open Folder..." => "باز کردن پوشه...",
        "Open URL..." => "باز کردن نشانی...",
        "Open from Clipboard" => "باز کردن از کلیپ‌بورد",
        "Load Subtitle..." => "بارگذاری زیرنویس...",
        "Load External Audio..." => "بارگذاری صدای خارجی...",
        "Undo Playlist Change" => "برگرداندن تغییر پلی‌لیست",
        "Redo Playlist Change" => "انجام مجدد تغییر پلی‌لیست",
        "Full Screen" => "تمام‌صفحه",
        "Playlist..." => "پلی‌لیست...",
        "Playback Information (OSD)" => "اطلاعات پخش (OSD)",
        "Playback/System Info..." => "اطلاعات پخش/سیستم...",
        "About..." => "درباره...",
        "Exit" => "خروج",
        // -- media info (OSD / info dialog) --------------------------------------------------------
        "No file is open." => "فایلی باز نیست.",
        "File" => "فایل",
        "Tracks" => "قطعه‌ها",
        "System" => "سیستم",
        "hardware" => "سخت‌افزاری",
        "software" => "نرم‌افزاری",
        "dropped" => "فریم افتاده",
        "Recent engine messages" => "پیام‌های اخیر موتور پخش",
        "Remove Bookmark" => "حذف نشانک",
        // -- new features: loop / sleep / history --------------------------------------------------
        "Loop" => "تکرار",
        "Loop: off" => "تکرار: خاموش",
        "Loop: current file" => "تکرار: فایل فعلی",
        "Loop: playlist" => "تکرار: پلی‌لیست",
        "Sleep Timer" => "تایمر خواب",
        "Sleep timer off" => "تایمر خواب خاموش شد",
        "Sleep after playlist" => "خواب بعد از پایان پلی‌لیست",
        "After Playlist" => "بعد از پلی‌لیست",
        "Paused by the sleep timer" => "با تایمر خواب متوقف شد",
        "min" => "دقیقه",
        "History" => "تاریخچه",
        "History..." => "تاریخچه...",
        "Nothing watched yet" => "هنوز چیزی تماشا نشده",
        "Clear History" => "پاک کردن تاریخچه",
        "Export Playlist..." => "خروجی پلی‌لیست...",
        "Playlist exported" => "پلی‌لیست ذخیره شد",
        "Could not export playlist" => "ذخیرهٔ پلی‌لیست ممکن نشد",
        "Mini Player" => "پلیر کوچک",
        // -- new features: online / disc / quality -------------------------------------------------
        "Open Disc (DVD/Blu-ray)..." => "باز کردن دیسک (DVD/بلوری)...",
        "Not a disc folder (VIDEO_TS / BDMV)" => "پوشهٔ دیسک نیست (VIDEO_TS / BDMV)",
        "Online Stream Quality" => "کیفیت پخش آنلاین",
        "Best Available" => "بهترین موجود",
        "4K (2160p)" => "4K (2160p)",
        "2K (1440p)" => "2K (1440p)",
        "Audio Only" => "فقط صدا",
        "Some audio and cache options apply after restart" => {
            "برخی گزینه‌های صدا و کش پس از راه‌اندازی مجدد اعمال می‌شوند"
        }
        // -- new features: video quality / filters / visualizer -----------------------------------
        "Motion & Sync" => "حرکت و همگام‌سازی",
        "Auto (default)" => "خودکار (پیش‌فرض)",
        "Audio Clock" => "ساعت صدا",
        "Display Sync (resample audio)" => "همگام با نمایشگر (نمونه‌برداری مجدد صدا)",
        "Display (drop/duplicate frames)" => "نمایشگر (حذف/تکرار فریم)",
        "Frame Interpolation" => "درون‌یابی فریم",
        "Image Scaling" => "مقیاس‌گذاری تصویر",
        "Bilinear" => "بیلینیار",
        "Bicubic" => "بای‌کیوبیک",
        "Spline36" => "اسپلاین۳۶",
        "Lanczos" => "لانچوس",
        "EWA Lanczos Sharp" => "EWA لانچوس تیز",
        "Filters" => "فیلترها",
        "Deband" => "حذف بندینگ",
        "Denoise" => "حذف نویز",
        "Sharpen" => "تیزبخشی",
        "Audio Visualizer" => "ویژوالایزر صدا",
        "Wave" => "موج",
        "Spectrum" => "طیف",
        // -- new features: audio / network preferences ---------------------------------------------
        "Network" => "شبکه",
        "Maximum volume (%)" => "حداکثر بلندی صدا (٪)",
        "Keep pitch when changing speed" => "حفظ زیر و بمی صدا هنگام تغییر سرعت",
        "Audio passthrough (S/PDIF)" => "عبور مستقیم صدا (S/PDIF)",
        "Exclusive audio output" => "خروجی صدای انحصاری",
        "Channel layout" => "چینش کانال‌ها",
        "Stereo" => "استریو",
        "Reversed stereo" => "استریو معکوس",
        "Default visualizer" => "ویژوالایزر پیش‌فرض",
        "Show visualizer for audio files" => "نمایش ویژوالایزر برای فایل‌های صوتی",
        "Stream cache (sec, 0 = auto)" => "کش استریم (ثانیه، ۰ = خودکار)",
        "Demuxer cache (MiB, 0 = auto)" => "کش دی‌ماکسر (مگابایت، ۰ = خودکار)",
        "HTTP user agent" => "یوزر-ایجنت HTTP",
        "HTTP headers" => "هدرهای HTTP",
        "One header per line, format: Key: Value" => "یک هدر در هر خط، با قالب: کلید: مقدار",
        "These apply to URLs; local files are unaffected" => {
            "این تنظیمات فقط برای نشانی‌های اینترنتی‌اند؛ فایل محلی تغییری نمی‌کند"
        }
        "None" => "هیچ",
        "AC3 (Dolby Digital)" => "AC3 (دالبی دیجیتال)",
        "DTS" => "DTS",
        "AC3 + DTS" => "AC3 + DTS",
        "E-AC3 + AC3 + DTS" => "E-AC3 + AC3 + DTS",
        "TrueHD + E-AC3 + AC3 + DTS" => "TrueHD + E-AC3 + AC3 + DTS",
        "Everything (DTS-HD, TrueHD, ...)" => "همه (DTS-HD، TrueHD و…)",
        // -- new features: subtitle style -------------------------------------------------------------
        "Subtitle Style" => "سبک زیرنویس",
        "Text color" => "رنگ متن",
        "Border width" => "پهنای حاشیه",
        "Border color" => "رنگ حاشیه",
        "Shadow" => "سایه",
        "Background box" => "زمینهٔ زیرنویس",
        "Background color" => "رنگ زمینه",
        "White" => "سفید",
        "Yellow" => "زرد",
        "Cyan" => "فیروزه‌ای",
        "Amber" => "کهربایی",
        "Black" => "سیاه",
        _ => key,
    }
}

#[cfg(test)]
mod tests {
    use super::{Lang, set_lang, tr};

    /// One sequential test: the active language is a process global, so parallel
    /// tests would race on it.
    #[test]
    fn translation_table_behavior() {
        set_lang(Lang::En);
        assert_eq!(tr("Playlist"), "Playlist");

        set_lang(Lang::Fa);
        assert_eq!(tr("Playlist"), "پلی‌لیست");
        assert_eq!(tr("Preferences"), "تنظیمات");
        assert_eq!(tr("No file is open."), "فایلی باز نیست.");
        assert_eq!(tr("Definitely not a real key"), "Definitely not a real key");
        set_lang(Lang::En);
        assert_eq!(tr("Playlist"), "Playlist");
    }
}
