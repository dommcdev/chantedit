PREFIX ?= $(HOME)/.local
APP_ID := dev.dominic.ChantEdit
CARGO_TARGET_DIR ?= target

.PHONY: build test install uninstall clean

build:
	cargo build --release --bins
	mkdir -p bin
	dir=$${CARGO_TARGET_DIR:-target}; cp $$dir/release/chantedit bin/

test:
	cargo test

install: build
	install -Dm755 bin/chantedit $(PREFIX)/bin/chantedit
	install -Dm644 data/$(APP_ID).desktop $(PREFIX)/share/applications/$(APP_ID).desktop
	install -Dm644 data/$(APP_ID).xml $(PREFIX)/share/mime/packages/$(APP_ID).xml
	sed -i 's|^Exec=.*|Exec=$(PREFIX)/bin/chantedit %f|' $(PREFIX)/share/applications/$(APP_ID).desktop
	-update-desktop-database $(PREFIX)/share/applications
	-update-mime-database $(PREFIX)/share/mime

uninstall:
	rm -f $(PREFIX)/bin/chantedit \
		$(PREFIX)/share/applications/$(APP_ID).desktop \
		$(PREFIX)/share/mime/packages/$(APP_ID).xml
	-update-desktop-database $(PREFIX)/share/applications
	-update-mime-database $(PREFIX)/share/mime

clean:
	rm -rf bin
	cargo clean
