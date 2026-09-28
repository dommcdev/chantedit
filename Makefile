PREFIX ?= $(HOME)/.local
APP_ID := dev.dominic.ChantEdit

.PHONY: build test install uninstall detect clean

build:
	go build -o bin/chantedit .
	go build -o bin/chantdetect ./cmd/chantdetect

test:
	go test ./...

install: build
	install -Dm755 bin/chantedit $(PREFIX)/bin/chantedit
	install -Dm644 data/$(APP_ID).desktop $(PREFIX)/share/applications/$(APP_ID).desktop
	sed -i 's|^Exec=.*|Exec=$(PREFIX)/bin/chantedit %f|' $(PREFIX)/share/applications/$(APP_ID).desktop

uninstall:
	rm -f $(PREFIX)/bin/chantedit $(PREFIX)/share/applications/$(APP_ID).desktop

# Visualise detection: make detect PDF="path/to/score.pdf"
detect: build
	./bin/chantdetect -out /tmp/chantdetect "$(PDF)"
	@echo "overlays written to /tmp/chantdetect"

clean:
	rm -rf bin
