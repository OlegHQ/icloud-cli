PREFIX ?= $(HOME)/.local

.PHONY: build install uninstall clean

build:
	cargo build --release

install: build
	install -d $(PREFIX)/bin
	install -m 755 target/release/icloud $(PREFIX)/bin/icloud

uninstall:
	rm -f $(PREFIX)/bin/icloud

clean:
	cargo clean
