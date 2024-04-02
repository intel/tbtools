# Thunderbolt/USB4 debug tools
# Copyright (C) 2023, Intel Corporation

CARGO = cargo
LN = ln
RM = rm

# Release build, uncomment for debug build
#CFLAGS =
#IFLAGS = --debug
CFLAGS = -r
IFLAGS =

# For buildroot, override $PREFIX if using something else
BR_HOME ?= $(HOME)/devel/buildroot
PREFIX ?= $(BR_HOME)/output/target/usr

build:
	$(CARGO) build $(CFLAGS)

run:
	$(CARGO) run $(CFLAGS)

install:
	$(CARGO) install $(IFLAGS) --path . --root $(PREFIX)
	# Create convenient lstb symlink as well
	$(LN) -sf tblist $(PREFIX)/bin/lstb

uninstall:
	$(CARGO) uninstall --root $(PREFIX)
	$(RM) -f $(PREFIX)/bin/lstb

clean:
	$(CARGO) clean

.PHONY: build run install clean
