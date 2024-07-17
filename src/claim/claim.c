// SPDX-License-Identifier: GPL-3.0-or-later

#include "claim.h"

// --------------------------------------------------------------------------------------------------------------------
// keep track of the last claiming failure reason

static const char *cloud_claim_failure_reason = NULL;

void claim_agent_failure_reason_set(const char *reason) {
    freez((void *)cloud_claim_failure_reason);
    cloud_claim_failure_reason = reason ? strdupz(reason) : NULL;
}

const char *claim_agent_failure_reason_get(void) {
    if(!cloud_claim_failure_reason)
        return "Agent is not claimed yet";
    else
        return cloud_claim_failure_reason;
}

// --------------------------------------------------------------------------------------------------------------------
// claimed_id load/save

bool claimed_id_save_to_file(const char *claimed_id_str) {
    bool ret;
    const char *filename = strdupz_path_subpath(netdata_configured_cloud_dir, "claimed_id");
    FILE *fp = fopen(filename, "w");
    if(fp) {
        fprintf(fp, "%s", claimed_id_str);
        fclose(fp);
        ret = true;
    }
    else {
        nd_log(NDLS_DAEMON, NDLP_ERR,
               "CLAIM: cannot open file '%s' for writing.", filename);
        ret = false;
    }

    freez((void *)filename);
    return ret;
}

static ND_UUID claimed_id_parse(const char *claimed_id, const char *source) {
    ND_UUID uuid;

    if(uuid_parse(claimed_id, uuid.uuid) != 0) {
        uuid = UUID_ZERO;
        nd_log(NDLS_DAEMON, NDLP_ERR,
               "CLAIM: claimed_id '%s' (loaded from '%s'), is not a valid UUID.",
               claimed_id, source);
    }

    return uuid;
}

static ND_UUID claimed_id_load_from_file(void) {
    ND_UUID uuid;

    long bytes_read;
    const char *filename = strdupz_path_subpath(netdata_configured_cloud_dir, "claimed_id");
    char *claimed_id = read_by_filename(filename, &bytes_read);

    if(!claimed_id)
        uuid = UUID_ZERO;
    else
        uuid = claimed_id_parse(claimed_id, filename);

    freez(claimed_id);
    freez((void *)filename);
    return uuid;
}

static ND_UUID claimed_id_get_from_cloud_conf(void) {
    if(appconfig_exists(&cloud_config, CONFIG_SECTION_GLOBAL, "claimed_id")) {
        const char *claimed_id = appconfig_get(&cloud_config, CONFIG_SECTION_GLOBAL, "claimed_id", "");
        if(claimed_id && *claimed_id)
            return claimed_id_parse(claimed_id, "cloud.conf");
    }
    return UUID_ZERO;
}

static ND_UUID claimed_id_load(void) {
    ND_UUID uuid = claimed_id_get_from_cloud_conf();
    if(UUIDeq(uuid, UUID_ZERO))
        uuid = claimed_id_load_from_file();

    return uuid;
}

// --------------------------------------------------------------------------------------------------------------------

/* Retrieve the claim id for the agent.
 * Caller owns the string.
*/
char *aclk_get_claimed_id()
{
    char *result;
    rrdhost_aclk_state_lock(localhost);
    result = (localhost->aclk_state.claimed_id == NULL) ? NULL : strdupz(localhost->aclk_state.claimed_id);
    rrdhost_aclk_state_unlock(localhost);
    return result;
}

/* Change the claimed state of the agent.
 *
 * This only happens when the user has explicitly requested it:
 *   - via the cli tool by reloading the claiming state
 *   - after spawning the claim because of a command-line argument
 * If this happens with the ACLK active under an old claim then we MUST KILL THE LINK
 */
bool load_claiming_state(void) {
    rrdhost_aclk_state_lock(localhost);
    if (localhost->aclk_state.claimed_id) {
        if (aclk_connected)
            localhost->aclk_state.prev_claimed_id = strdupz(localhost->aclk_state.claimed_id);
        freez(localhost->aclk_state.claimed_id);
        localhost->aclk_state.claimed_id = NULL;
    }

    if (aclk_connected) {
        nd_log(NDLS_DAEMON, NDLP_ERR,
               "CLAIM: agent was already connected to NC - forcing reconnection under new credentials");
        aclk_kill_link = 1;
    }
    aclk_disable_runtime = 0;

    ND_UUID uuid = claimed_id_load();
    if(UUIDeq(uuid, UUID_ZERO)) {
        // not found
        if(claim_agent_automatically())
            uuid = claimed_id_load();
    }

    bool have_claimed_id = false;
    if(!UUIDeq(uuid, UUID_ZERO)) {
        // we go it somehow
        localhost->aclk_state.claimed_id = mallocz(UUID_STR_LEN);
        uuid_unparse_lower(uuid.uuid, localhost->aclk_state.claimed_id);
        have_claimed_id = true;
    }

    rrdhost_aclk_state_unlock(localhost);
    invalidate_node_instances(&localhost->host_uuid, have_claimed_id ? &uuid.uuid : NULL);
    metaqueue_store_claim_id(&localhost->host_uuid, have_claimed_id ? &uuid.uuid : NULL);

    errno_clear();

    if (!have_claimed_id)
        nd_log(NDLS_DAEMON, NDLP_ERR,
               "CLAIM: Unable to find our claimed_id, setting state to AGENT_UNCLAIMED");
    else
        nd_log(NDLS_DAEMON, NDLP_INFO,
               "CLAIM: Found a valid claimed_id, setting state to AGENT_CLAIMED");

    return have_claimed_id;
}

CLOUD_STATUS claim_reload_and_wait_online(void) {
    nd_log(NDLS_DAEMON, NDLP_INFO,
           "CLAIM: Reloading Agent Claiming configuration.");

    nd_log_limits_unlimited();
    cloud_conf_load(0);
    bool claimed = load_claiming_state();
    registry_update_cloud_base_url();
    rrdpush_send_claimed_id(localhost);
    nd_log_limits_reset();

    CLOUD_STATUS status = cloud_status();
    if(claimed) {
        int ms = 0;
        do {
            status = cloud_status();
            if (status == CLOUD_STATUS_ONLINE && __atomic_load_n(&localhost->node_id, __ATOMIC_RELAXED))
                break;

            sleep_usec(50 * USEC_PER_MS);
            ms += 50;
        } while (ms < 10000);
    }

    return status;
}
