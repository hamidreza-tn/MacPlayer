CARGO ?= cargo

.PHONY: check test build package-macos package-macos-universal docker-build

check:
	$(CARGO) check --offline --locked --message-format=short

test:
	$(CARGO) test --offline --locked

build:
	$(CARGO) build --release --offline --locked

package-macos:
	@test -n "$(ARCH)" || (printf '%s\n' 'usage: make package-macos ARCH=arm64|x86_64' >&2; exit 2)
	bash scripts/package-macos.sh "$(ARCH)"

package-macos-universal:
	@test -n "$(ARM_APP)" -a -n "$(X86_APP)" -a -n "$(OUT)" || (printf '%s\n' 'usage: make package-macos-universal ARM_APP=/path/arm64/MacPlayer.app X86_APP=/path/x86_64/MacPlayer.app OUT=dist/macos-universal' >&2; exit 2)
	bash scripts/package-macos-universal.sh "$(ARM_APP)" "$(X86_APP)" "$(OUT)"

docker-build:
	docker build --tag macplayer:latest .
