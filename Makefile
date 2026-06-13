PREFIX ?= $(HOME)/.local

.PHONY: build check hooks install uninstall clean

build:
	cargo build --release

check:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo test --workspace --locked

hooks:
	./scripts/install-hooks.sh

install: build
	install -d $(PREFIX)/bin
	install -m 755 target/release/icloud $(PREFIX)/bin/icloud

uninstall:
	rm -f $(PREFIX)/bin/icloud

clean:
	cargo clean
