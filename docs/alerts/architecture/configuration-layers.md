# 13.2 Configuration Layers

Netdata supports multiple configuration layers for health alerts. Understanding precedence rules helps in making modifications that take effect as intended.

## Stock Configuration Layer

Stock alerts are distributed with Netdata and reside in `/usr/lib/netdata/conf.d/health.d/`. These files are installed by the Netdata package and updated with each release. Modifying stock files is not recommended because changes are overwritten during upgrades.

Stock configurations define the default alert set. User configuration files load first. When a user configuration file exists with the same filename as a stock file, the stock file is excluded entirely—it is not loaded at all.

## Custom Configuration Layer

Custom alerts reside in `/etc/netdata/health.d/`. Files in this directory take precedence over stock configurations for the same alert names.

An alert defined in `/etc/netdata/health.d/` with the same name as a stock alert replaces the stock definition entirely.

## Cloud Configuration Layer

Netdata Cloud can define alerts through the Alerts Configuration Manager. These Cloud-defined alerts take precedence over both stock and custom layers.

Cloud-defined alerts are stored remotely and synchronized to Agents on demand.

## Configuration File Loading

At startup and after configuration reload (`netdatacli reload-health`), Netdata loads all configuration files from user and stock directories.

To view currently loaded alerts, use the API endpoint `/api/v1/alarms?all` or check `/var/log/netdata/error.log` for any configuration parsing errors.