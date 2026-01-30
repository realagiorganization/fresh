SHELL := /usr/bin/env bash

CARGO ?= cargo
PACKAGE ?= fresh-editor
BIN_NAME ?= fresh
BIN ?= ./target/release/$(BIN_NAME)

FRESH ?= fresh

TMUX ?= tmux
TMUX_SESSION ?= fresh

ARGS ?=

.PHONY: help
help:
	@printf '%s\n' \
		"Fresh developer commands" \
		"" \
		"Build:" \
		"  make build            Build release binary (target/release/fresh)" \
		"  make run ARGS=\"...\"   Run built binary" \
		"  make install          Install via cargo install --path (from this repo)" \
		"  make install-system   Install via scripts/install.sh (.deb/.rpm/AppImage/etc)" \
		"  make run-installed    Run system-installed 'fresh'" \
		"" \
		"tmux:" \
		"  make tmux-start       Start Fresh in a named tmux session (detached)" \
		"  make tmux-attach      Attach to the tmux session" \
		"  make tmux-kill        Kill the tmux session" \
		"  make tmux-smoke       Launch Fresh in tmux and quit (Ctrl+Q)" \
		"  make tmux-start-installed  Start system-installed fresh in tmux" \
		"  make tmux-smoke-installed  tmux smoke using system-installed fresh" \
		"" \
		"Docker:" \
		"  make docker-build     Build Docker image (fresh:local)" \
		"  make docker-run       Run Docker image interactively" \
		"" \
		"Vars:" \
		"  TMUX_SESSION=$(TMUX_SESSION)  ARGS=\"$(ARGS)\""

.PHONY: build
build:
	$(CARGO) build --release -p $(PACKAGE)
	@test -x $(BIN)

.PHONY: run
run: build
	$(BIN) $(ARGS)

.PHONY: install
install:
	$(CARGO) install --path crates/fresh-editor --locked

.PHONY: install-system
install-system:
	sh ./scripts/install.sh

.PHONY: run-installed
run-installed:
	$(FRESH) $(ARGS)

.PHONY: tmux-start
tmux-start: build
	$(TMUX) has-session -t "$(TMUX_SESSION)" 2>/dev/null || \
		$(TMUX) new-session -d -s "$(TMUX_SESSION)" "bash -lc 'stty -ixon; exec $(BIN) $(ARGS)'"
	@printf '%s\n' "Started tmux session: $(TMUX_SESSION)" "Attach with: tmux attach -t $(TMUX_SESSION)"

.PHONY: tmux-attach
tmux-attach:
	$(TMUX) attach -t "$(TMUX_SESSION)"

.PHONY: tmux-kill
tmux-kill:
	$(TMUX) kill-session -t "$(TMUX_SESSION)" 2>/dev/null || true

.PHONY: tmux-smoke
tmux-smoke: build
	./scripts/tmux_smoke.sh "$(BIN) --no-session --no-upgrade-check" "$(TMUX_SESSION)-smoke"

.PHONY: tmux-start-installed
tmux-start-installed:
	$(TMUX) has-session -t "$(TMUX_SESSION)" 2>/dev/null || \
		$(TMUX) new-session -d -s "$(TMUX_SESSION)" "bash -lc 'stty -ixon; exec $(FRESH) $(ARGS)'"
	@printf '%s\n' "Started tmux session: $(TMUX_SESSION)" "Attach with: tmux attach -t $(TMUX_SESSION)"

.PHONY: tmux-smoke-installed
tmux-smoke-installed:
	./scripts/tmux_smoke.sh "$(FRESH) --no-session --no-upgrade-check" "$(TMUX_SESSION)-smoke"

.PHONY: docker-build
docker-build:
	docker build -t fresh:local .

.PHONY: docker-run
docker-run:
	docker run --rm -it -v "$(CURDIR):/work" -w /work fresh:local $(ARGS)
