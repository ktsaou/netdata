// SPDX-License-Identifier: GPL-3.0-or-later

#include "claim.h"

struct config cloud_config = {
    .first_section = NULL,
    .last_section = NULL,
    .mutex = NETDATA_MUTEX_INITIALIZER,
    .index = {
        .avl_tree = {
            .root = NULL,
            .compar = appconfig_section_compare
        },
        .rwlock = AVL_LOCK_INITIALIZER
    }
};

const char *cloud_config_url_get(void) {
    return appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "url", DEFAULT_CLOUD_BASE_URL);
}

void cloud_config_url_set(const char *url) {
    if(!url || *url) return;

    const char *existing = cloud_config_url_get();
    if(strcmp(existing, url) != 0)
        appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "url", url);
}

const char *cloud_config_proxy_get(void) {
    // load cloud.conf or internal default
    const char *proxy = appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "proxy", "env");

    // backwards compatibility, from when proxy was in netdata.conf
    // netdata.conf has bigger priority
    if (config_exists(CONFIG_SECTION_CLOUD, "proxy")) {
        // get it from netdata.conf
        proxy = config_get(CONFIG_SECTION_CLOUD, "proxy", proxy);

        // update cloud.conf
        proxy = appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "proxy", proxy);
    }
    else {
        // set in netdata.conf the proxy of cloud.conf
        config_set(CONFIG_SECTION_CLOUD, "proxy", proxy);
    }

    return proxy;
}

bool cloud_config_insecure_get(void) {
    // load it from cloud.conf or use internal default
    return appconfig_get_boolean(&cloud_config, CONFIG_SECTION_GLOBAL, "insecure", CONFIG_BOOLEAN_NO);
}

static void cloud_conf_load_defaults(void) {
    appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "url", DEFAULT_CLOUD_BASE_URL);
    appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "proxy", "env");
    appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "token", "");
    appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "rooms", "");
    appconfig_get_boolean(&cloud_config, CONFIG_SECTION_GLOBAL, "insecure", CONFIG_BOOLEAN_NO);
    appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "machine_guid", "");
    appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "claimed_id", "");
    appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "hostname", "");
}

void cloud_conf_load(int silent) {
    errno_clear();
    char *filename = strdupz_path_subpath(netdata_configured_cloud_dir, "cloud.conf");
    int ret = appconfig_load(&cloud_config, filename, 1, NULL);

    if(!ret && !silent)
        nd_log(NDLS_DAEMON, NDLP_ERR,
               "CLAIM: cannot load cloud config '%s'. Running with internal defaults.", filename);

    freez(filename);

    appconfig_move(&cloud_config,
                   CONFIG_SECTION_GLOBAL, "cloud base url",
                   CONFIG_SECTION_GLOBAL, "url");

    cloud_conf_load_defaults();
}

void cloud_conf_init_after_registry(void) {
    const char *machine_guid = appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "machine_guid", "");
    const char *hostname = appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "hostname", "");

    // for machine guid and hostname we have to use appconfig_set() for that they will be saved uncommented
    if(!machine_guid || !*machine_guid)
        appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "machine_guid", registry_get_this_machine_guid());

    if(!hostname || !*hostname)
        appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "hostname", registry_get_this_machine_hostname());
}

bool cloud_conf_save(void) {
    char filename[FILENAME_MAX + 1];

    CLEAN_BUFFER *wb = buffer_create(0, NULL);
    appconfig_generate(&cloud_config, wb, false, false);
    snprintfz(filename, sizeof(filename), "%s/cloud.conf", netdata_configured_cloud_dir);
    FILE *fp = fopen(filename, "w");
    if(!fp) {
        nd_log(NDLS_DAEMON, NDLP_ERR, "Cannot open file '%s' for writing.", filename);
        return false;
    }

    fprintf(fp, "%s", buffer_tostring(wb));
    fclose(fp);
    return true;
}

bool cloud_conf_regenerate(const char *claimed_id_str, const char *machine_guid, const char *hostname, const char *token, const char *rooms, const char *url, const char *proxy, int insecure) {
    // for backwards compatibility (older agents), save the claimed_id to its file
    claimed_id_save_to_file(claimed_id_str);

    appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "url", url);
    appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "proxy", proxy ? proxy : "");
    appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "token", token ? token : "");
    appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "rooms", rooms ? rooms : "");
    appconfig_set_boolean(&cloud_config, CONFIG_SECTION_GLOBAL, "insecure", insecure);
    appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "machine_guid", machine_guid ? machine_guid : "");
    appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "claimed_id", claimed_id_str ? claimed_id_str : "");
    appconfig_set(&cloud_config, CONFIG_SECTION_GLOBAL, "hostname", hostname ? hostname : "");

    return cloud_conf_save();
}