// SPDX-License-Identifier: GPL-3.0-or-later

#include "apps_plugin.h"

// ----------------------------------------------------------------------------
// apps_groups.conf
// aggregate all processes in groups, to have a limited number of dimensions

struct target *get_users_target(uid_t uid) {
    struct target *w;
    for(w = users_root_target ; w ; w = w->next)
        if(w->uid == uid) return w;

    w = callocz(sizeof(struct target), 1);
    {
        char buf[50];
        snprintfz(buf, sizeof(buf), "%u", uid);

        w->compare = string_strdupz(buf);
        w->id = string_dup(w->compare);
    }

    apps_username_get_from_uid(uid, w->name, sizeof(w->name));
    strncpyz(w->clean_name, w->name, sizeof(w->clean_name) - 1);
    netdata_fix_chart_name(w->clean_name);

    w->uid = uid;

    w->next = users_root_target;
    users_root_target = w;

    debug_log("added uid %u ('%s') target", w->uid, w->name);

    return w;
}

struct target *get_groups_target(gid_t gid) {
    struct target *w;
    for(w = groups_root_target ; w ; w = w->next)
        if(w->gid == gid) return w;

    w = callocz(sizeof(struct target), 1);
    {
        char buf[50];
        snprintfz(buf, sizeof(buf), "%u", gid);

        w->compare = string_strdupz(buf);
        w->id = string_dup(w->compare);
    }

    apps_groupname_get_from_gid(gid, w->name, sizeof(w->name));

    strncpyz(w->clean_name, w->name, sizeof(w->name) - 1);
    netdata_fix_chart_name(w->clean_name);

    w->gid = gid;

    w->next = groups_root_target;
    groups_root_target = w;

    debug_log("added gid %u ('%s') target", w->gid, w->name);

    return w;
}

// find or create a new target
// there are targets that are just aggregated to other target (the second argument)
static struct target *get_apps_groups_target(const char *id, struct target *target, const char *name) {
    int tdebug = 0, thidden = target?target->hidden:0, ends_with = 0;
    const char *nid = id;

    // extract the options
    while(nid[0] == '-' || nid[0] == '+' || nid[0] == '*') {
        if(nid[0] == '-') thidden = 1;
        if(nid[0] == '+') tdebug = 1;
        if(nid[0] == '*') ends_with = 1;
        nid++;
    }

    STRING *nidstr = string_strdupz(nid);

    // find if it already exists
    struct target *w, *last = apps_groups_root_target;
    for(w = apps_groups_root_target ; w ; w = w->next) {
        if(nidstr == w->id)
            return w;

        last = w;
    }

    // find an existing target
    if(unlikely(!target)) {
        while(*name == '-') {
            if(*name == '-') thidden = 1;
            name++;
        }

        for(target = apps_groups_root_target ; target != NULL ; target = target->next) {
            if(!target->target && strcmp(name, target->name) == 0)
                break;
        }

        if(unlikely(debug_enabled)) {
            if(unlikely(target))
                debug_log("REUSING TARGET NAME '%s' on ID '%s'", target->name, string2str(target->id));
            else
                debug_log("NEW TARGET NAME '%s' on ID '%s'", name, id);
        }
    }

    if(target && target->target)
        fatal("Internal Error: request to link process '%s' to target '%s' which is linked to target '%s'",
            id, string2str(target->id), string2str(target->target->id));

    w = callocz(sizeof(struct target), 1);
    w->id = string_strdupz(nid);

    if(unlikely(!target))
        // copy the name
        strncpyz(w->name, name, sizeof(w->name));
    else
        // copy the id
        strncpyz(w->name, nid, sizeof(w->name));

    // dots are used to distinguish chart type and id in streaming, so we should replace them
    strncpyz(w->clean_name, w->name, sizeof(w->name) - 1);
    netdata_fix_chart_name(w->clean_name);
    for (char *d = w->clean_name; *d; d++) {
        if (*d == '.')
            *d = '_';
    }

    {
        size_t len = strlen(nid);
        char buf[len + 1];
        memcpy(buf, nid, sizeof(buf));
        if(buf[len - 1] == '*') {
            buf[len - 1] = '\0';
            w->starts_with = 1;
        }
        w->ends_with = ends_with;
        w->compare = string_strdupz(buf);
    }

    if(w->starts_with && w->ends_with)
        proc_pid_cmdline_is_needed = true;

    w->hidden = thidden;
#ifdef NETDATA_INTERNAL_CHECKS
    w->debug_enabled = tdebug;
#else
    if(tdebug)
        fprintf(stderr, "apps.plugin has been compiled without debugging\n");
#endif
    w->target = target;

    // append it, to maintain the order in apps_groups.conf
    if(last) last->next = w;
    else apps_groups_root_target = w;

    debug_log("ADDING TARGET ID '%s', process name '%s' (%s), aggregated on target '%s', options: %s %s"
              , string2str(w->id)
              , string2str(w->compare)
              , (w->starts_with && w->ends_with)?"substring":((w->starts_with)?"prefix":((w->ends_with)?"suffix":"exact"))
              , w->target?w->target->name:w->name
              , (w->hidden)?"hidden":"-"
              , (w->debug_enabled)?"debug":"-"
    );

    return w;
}

// read the apps_groups.conf file
int read_apps_groups_conf(const char *path, const char *file) {
    char filename[FILENAME_MAX + 1];

    snprintfz(filename, sizeof(filename), "%s/apps_%s.conf", path, file);

    debug_log("process groups file: '%s'", filename);

    // ----------------------------------------

    procfile *ff = procfile_open(filename, " :\t", PROCFILE_FLAG_DEFAULT);
    if(!ff) return 1;

    procfile_set_quotes(ff, "'\"");

    ff = procfile_readall(ff);
    if(!ff)
        return 1;

    size_t line, lines = procfile_lines(ff);

    for(line = 0; line < lines ;line++) {
        size_t word, words = procfile_linewords(ff, line);
        if(!words) continue;

        char *name = procfile_lineword(ff, line, 0);
        if(!name || !*name) continue;

        // find a possibly existing target
        struct target *w = NULL;

        // loop through all words, skipping the first one (the name)
        for(word = 0; word < words ;word++) {
            char *s = procfile_lineword(ff, line, word);
            if(!s || !*s) continue;
            if(*s == '#') break;

            // is this the first word? skip it
            if(s == name) continue;

            // add this target
            struct target *n = get_apps_groups_target(s, w, name);
            if(!n) {
                netdata_log_error("Cannot create target '%s' (line %zu, word %zu)", s, line, word);
                continue;
            }

            // just some optimization
            // to avoid searching for a target for each process
            if(!w) w = n->target?n->target:n;
        }
    }

    procfile_close(ff);

    apps_groups_default_target = get_apps_groups_target("p+!o@w#e$i^r&7*5(-i)l-o_", NULL, "other"); // match nothing
    if(!apps_groups_default_target)
        fatal("Cannot create default target");
    apps_groups_default_target->is_other = true;

    // allow the user to override group 'other'
    if(apps_groups_default_target->target)
        apps_groups_default_target = apps_groups_default_target->target;

    return 0;
}
