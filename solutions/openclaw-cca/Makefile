CC      := gcc
CFLAGS  := -O2 -Wall -Wextra
DESTDIR ?=

BIN_DIR  := $(DESTDIR)/usr/local/bin
SBIN_DIR := $(DESTDIR)/usr/local/sbin
UNIT_DIR := $(DESTDIR)/etc/systemd/system

.PHONY: all install uninstall clean

all: extend-rem3

extend-rem3: src/extend_tools/attest.c
	$(CC) $(CFLAGS) -o $@ $<

install: all
	install -d $(BIN_DIR) $(SBIN_DIR) $(UNIT_DIR)
	install -m 0550 extend-rem3           $(BIN_DIR)/extend-rem3
	install -m 0550 scripts/openclaw-rbc-unlock.sh    $(SBIN_DIR)/openclaw-rbc-unlock.sh
	install -m 0550 scripts/openclaw-init.sh          $(SBIN_DIR)/openclaw-init.sh
	install -m 0550 scripts/openclaw-create-volume.sh $(SBIN_DIR)/openclaw-create-volume.sh
	install -m 0640 systemd/openclaw-luks-unlock.service $(UNIT_DIR)/openclaw-luks-unlock.service

uninstall:
	rm -f $(BIN_DIR)/extend-rem3
	rm -f $(SBIN_DIR)/openclaw-rbc-unlock.sh
	rm -f $(SBIN_DIR)/openclaw-init.sh
	rm -f $(SBIN_DIR)/openclaw-create-volume.sh
	rm -f $(UNIT_DIR)/openclaw-luks-unlock.service

clean:
	rm -f extend-rem3
