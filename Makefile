# Thunderbolt/USB4 debug tools
# Copyright (C) 2023, Intel Corporation

CARGO = cargo
INSTALL = install
LN = ln
RM = rm
RMDIR = rmdir

# Release build, uncomment for debug build
#CFLAGS =
#IFLAGS = --debug
CFLAGS = -r
IFLAGS =

# For buildroot, override $PREFIX if using something else
BR_HOME ?= $(HOME)/devel/buildroot
PREFIX ?= $(BR_HOME)/output/target/usr

TOOLS = tbadapters tbauth tbdump tbget tblist tbmargin tbmonitor tbset tbtunnels tbtrace
SCRIPTS = nvm-version.sh pcie-downstream-mapping.sh reset-port.sh tb-bandwidth.sh

build:
	$(CARGO) build $(CFLAGS)

run:
	$(CARGO) run $(CFLAGS)

install-scripts:
	$(INSTALL) -d $(PREFIX)/share/tbtools/scripts
	$(foreach script, $(SCRIPTS), $(INSTALL) -m 0755 scripts/$(script) $(PREFIX)/share/tbtools/scripts/$(script);)

uninstall-scripts:
	$(foreach script, $(SCRIPTS), $(RM) -f $(PREFIX)/share/tbtools/scripts/$(script);)
	-$(RMDIR) $(PREFIX)/share/tbtools/scripts

install-completion:
	$(INSTALL) -d $(PREFIX)/share/bash-completion/completions
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
