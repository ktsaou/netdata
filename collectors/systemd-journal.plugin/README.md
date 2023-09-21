# systemd-journal.plugin

The plugin uses the shared systemd library to access systemd journal logs.


## Speed

We have made a [PR to systemd](https://github.com/systemd/systemd/pull/29261) to improve its performance by a factor of 4.

To test netdata with that PR, you need to go through these steps:

1. Clone it: `git clone https://github.com/ktsaou/systemd.git systemd-ktsaou.git`
2. Step into it: `cd systemd-ktsaou.git`
3. Build it: `rm -rf build; meson --buildtype=debugoptimized build . && ninja -C build version.h && ninja -v -C build libsystemd.a`
4. Step into netdata: `cd ../netdata.git`
5. Edit `Makefile.am` and replace the `if ENABLE_PLUGIN_SYSTEMD_JOURNAL` block with the following:

```
if ENABLE_PLUGIN_SYSTEMD_JOURNAL
    plugins_PROGRAMS += systemd-journal.plugin
    systemd_journal_plugin_SOURCES = $(SYSTEMD_JOURNAL_PLUGIN_FILES)
    systemd_journal_plugin_LDADD = \
        /path/to/systemd-ktsaou.git/build/libsystemd.a \
        -lzstd -lcap -lgcrypt -llzma \
        $(NETDATA_COMMON_LIBS) \
        $(OPTIONAL_SYSTEMD_LIBS) \
        $(NULL)
endif
```

In the above, replace `/path/to/systemd-ktsaou.git/build/libsystemd.a` with the right absolute path for your case.

Finally, edit `systemd-journal.c` and uncomment the 2 lines referencing `sd_journal_enable_fast_query`.


Finally, compile and install netdata with this:

```
CFLAGS="-O2 -g -no-pie" ./netdata-installer.sh
```

With these changes, systemd-journal.plugin is about twice as fast (vs itself), and twice faster compared to `journalctl` for the same query.
It seems that `journalctl` needs a lot more improvements to perform well...

