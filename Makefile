# Thunderbolt/USB4 debug tools
# Copyright (C) 2023, Intel Corporation

CARGO = cargo
INSTALL = install
LN = ln
RM = rm
MKDIR = mkdir

# Release build, uncomment for debug build
#CFLAGS =
#IFLAGS = --debug
CFLAGS = -r
IFLAGS =

# For buildroot, override $PREFIX if using something else
BR_HOME ?= $(HOME)/devel/buildroot
PREFIX ?= $(BR_HOME)/output/target/usr

TOOLS = tbadapters tbauth tbdump tbget tblist tbmargin tbset tbtrace

build:
	$(CARGO) build $(CFLAGS)

run:
	$(CARGO) run $(CFLAGS)

install-scripts:
	$(MKDIR) -p $(PREFIX)/share/tbtools/scripts
	$(INSTALL) -m 0755 scripts/alloc-bw.sh $(PREFIX)/share/tbtools/scripts/
	$(INSTALL) -m 0755 scripts/disable-bw.sh $(PREFIX)/share/tbtools/scripts/
	$(INSTALL) -m 0755 scripts/dump-dpcd.sh $(PREFIX)/share/tbtools/scripts/
	$(INSTALL) -m 0755 scripts/enable-bw.sh $(PREFIX)/share/tbtools/scripts/
	$(INSTALL) -m 0755 scripts/estimated-bw.sh $(PREFIX)/share/tbtools/scripts/
	$(INSTALL) -m 0755 scripts/nvm-version.sh $(PREFIX)/share/tbtools/scripts/
	$(INSTALL) -m 0755 scripts/reset-port.sh $(PREFIX)/share/tbtools/scripts/
	$(INSTALL) -m 0755 scripts/tb-bandwidth.sh $(PREFIX)/share/tbtools/scripts/

uninstall-scripts:
	$(RM) -rf $(PREFIX)/share/tbtools/scripts

install-completion:
	$(INSTALL) -m 0644 scripts/tbtools-completion.bash $(PREFIX)/share/bash-completion/completions
	$(foreach tool, $(TOOLS), $(LN) -sf tbtools-completion.bash $(PREFIX)/share/bash-completion/completions/$(tool);)

uninstall-completion:
	$(foreach tool, $(TOOLS), $(RM) -f $(PREFIX)/share/bash-completion/completions/$(tool);)
	$(RM) -f $(PREFIX)/share/bash-completion/completions/tbtools-completion.bash

install-binaries:
	$(CARGO) install $(IFLAGS) --path . --root $(PREFIX)
	# Create convenient lstb symlink as well
	$(LN) -sf tblist $(PREFIX)/bin/lstb

uninstall-binaries:
	$(CARGO) uninstall --root $(PREFIX)
	$(RM) -f $(PREFIX)/bin/lstb

install: install-binaries install-completion install-scripts

uninstall: uninstall-scripts uninstall-completion uninstall-binaries

clean:
	$(CARGO) clean

.PHONY: build run install clean
