// SPDX-License-Identifier: GPL-3.0-or-later

#include "mcp-tools-alert-transitions.h"
#include "mcp-params.h"
#include "database/contexts/rrdcontext.h"

// Schema for alert transitions
void mcp_tool_list_alert_transitions_schema(BUFFER *buffer) {
    // Tool metadata
    buffer_json_member_add_object(buffer, "inputSchema");
    buffer_json_member_add_string(buffer, "type", "object");
    buffer_json_member_add_string(buffer, "title", "List alert transitions");
    
    buffer_json_member_add_object(buffer, "properties");
    
    // Time range
    mcp_schema_add_time_params(
        buffer,
        "alert transitions",
        false);

    mcp_schema_add_array_param(
        buffer, "alerts",
        "Filter by alert names",
        "Array of specific alert names to filter by. "
        "Each alert name must be an exact match - no wildcards or patterns allowed. "
        "Use '" MCP_TOOL_LIST_ALL_ALERTS "' to discover available alert names. "
        "If not specified, all alerts are included. "
        "Examples: [\"disk_space_usage\", \"cpu_iowait\", \"ram_in_use\"]");

    // Nodes filter
    mcp_schema_add_array_param(
        buffer, "nodes", "Filter nodes",
        "Show only alerts transitions for these nodes.\n"
        "Use 'list_nodes' to discover available nodes.\n"
        "If not specified, alerts transitions from all nodes are included. "
        "Examples: [\"node1\", \"node2\"], [\"web-server-01\", \"db-server-01\"]");
    
    mcp_schema_add_array_param(
        buffer, "metrics",
        "Filter by metrics",
        "Array of specific metric names to filter by. "
        "Each metric must be an exact match - no wildcards or patterns allowed. "
        "Use '" MCP_TOOL_LIST_METRICS "' to discover available metrics. "
        "If not specified, all metrics are included. "
        "Examples: [\"system.cpu\", \"system.load\"], [\"disk.io\", \"disk.space\"]");

    mcp_schema_add_array_param(
        buffer, "instances",
        "Filter by instances",
        "Query only the given instances.\n"
        "Use the '" MCP_TOOL_GET_METRICS_DETAILS "' tool to discover available instances for a metric.\n"
        "If no instances are specified, all instances of the metric are queried.\n"
        "Example: [\"instance1\", \"instance2\", \"instance3\"]\n."
        "IMPORTANT: when you have a choice, prefer to filter by labels instead of instances, because many monitored "
        "components may change instance names over time.");

    // Status filter (required multi-select enum)
    buffer_json_member_add_object(buffer, "status");
    {
        buffer_json_member_add_string(buffer, "type", "array");
        buffer_json_member_add_string(buffer, "title", "Filter by status");
        buffer_json_member_add_string(
            buffer, "description",
            "Select the alert statuses of interest. At least one status must be selected.\n"
            " - CRITICAL: the highest severity, indicates a critical issue that needs immediate attention.\n"
            " - WARNING: indicates a potential issue that should be monitored but is not critical.\n"
            " - CLEAR: the normal state state for alerts, indicating that the alert is not triggered.\n"
            " - UNDEFINED: the alerts failed to be evaluated (some variable of it is undefined, division by zero, etc).\n"
            " - UNINITIALIZED: the alert has not been initialized for the first time yet, no data available.\n"
            " - REMOVED: the alert was removed (happens during netdata shutdown, child disconnect, health reload).\n"
            "Multiple statuses can be selected. Example: [\"CRITICAL\", \"WARNING\"]");
        
        // Define items schema with enum values
        buffer_json_member_add_object(buffer, "items");
        {
            buffer_json_member_add_string(buffer, "type", "string");
            buffer_json_member_add_array(buffer, "enum");
            buffer_json_add_array_item_string(buffer, "CRITICAL");
            buffer_json_add_array_item_string(buffer, "WARNING");
            buffer_json_add_array_item_string(buffer, "CLEAR");
            buffer_json_add_array_item_string(buffer, "UNDEFINED");
            buffer_json_add_array_item_string(buffer, "UNINITIALIZED");
            buffer_json_add_array_item_string(buffer, "REMOVED");
            buffer_json_array_close(buffer);
        }
        buffer_json_object_close(buffer); // items
    }
    buffer_json_object_close(buffer); // status
    
    mcp_schema_add_array_param(
        buffer, "classifications",
        "Filter by classifications", 
        "Array of specific alert classifications to filter by. "
        "Each classification must be an exact match - no wildcards or patterns allowed. "
        "Use '" MCP_TOOL_LIST_ALL_ALERTS "' to discover available classifications. "
        "If not specified, all classifications are included. "
        "Examples: [\"Errors\", \"Latency\", \"Utilization\"]");
    
    mcp_schema_add_array_param(
        buffer, "types",
        "Filter by types",
        "Array of specific alert types to filter by. "
        "Each type must be an exact match - no wildcards or patterns allowed. "
        "Use '" MCP_TOOL_LIST_ALL_ALERTS "' to discover available types. "
        "If not specified, all types are included. "
        "Examples: [\"System\", \"Web Server\", \"Database\"]");
    
    mcp_schema_add_array_param(
        buffer, "components",
        "Filter by components",
        "Array of specific components to filter by. "
        "Each component must be an exact match - no wildcards or patterns allowed. "
        "Use '" MCP_TOOL_LIST_ALL_ALERTS "' to discover available components. "
        "If not specified, all components are included. "
        "Examples: [\"Network\", \"Disk\", \"Memory\"]");
    
    mcp_schema_add_array_param(
        buffer, "roles",
        "Filter by roles",
        "Array of specific roles to filter by. "
        "Each role must be an exact match - no wildcards or patterns allowed. "
        "Use '" MCP_TOOL_LIST_ALL_ALERTS "' to discover available roles. "
        "If not specified, all roles are included. "
        "Examples: [\"sysadmin\", \"webmaster\", \"dba\"]");
    
    // Cardinality limit
    mcp_schema_add_cardinality_limit(
        buffer,
        "Number of most recent alert transitions to return",
        MCP_ALERTS_CARDINALITY_LIMIT,  // default value
        1,  // minimum
        MAX(MCP_ALERTS_CARDINALITY_LIMIT, MCP_ALERTS_CARDINALITY_LIMIT_MAX));  // max value

    // Pagination cursor
    mcp_schema_add_string_param(buffer, "cursor",
        "Pagination cursor",
        "Pagination cursor from previous response. Use the 'nextCursor' value from the previous response to get the next page of results.",
        NULL, false);
    
    // Timeout parameter
    mcp_schema_add_timeout(buffer, "timeout",
        "Query timeout",
        "Maximum time to wait for the query to complete (in seconds)",
        60, 1, 3600, false);
    
    buffer_json_object_close(buffer); // properties
    
    // Required fields
    buffer_json_member_add_array(buffer, "required");
    buffer_json_add_array_item_string(buffer, "status");
    buffer_json_array_close(buffer);
    
    buffer_json_object_close(buffer); // inputSchema
}

#include "core/mcp-core.h" // For mcp_job_add_error_response, mcp_job_add_json_response
#include "core/mcp-job.h"  // For MCP_REQ_JOB

// Execute alert transitions query
int mcp_tool_list_alert_transitions(MCP_REQ_JOB *job) { // MCP_RETURN_CODE mcp_tool_list_alert_transitions_execute(MCP_CLIENT *mcpc, struct json_object *params, MCP_REQUEST_ID id) {
    if (!job) { // Simplified check, assumes job->params might be NULL if no params are sent
        return -1; // Should be caught by MCP core ideally
    }

    // USER_AUTH *auth = job->auth; // Not directly used by rrdcontext_to_json_v2 for this call
    struct json_object *params = job->params;
    CLEAN_BUFFER *error_buffer = buffer_create(256);
    
    // Extract nodes array
    const char *nodes_pattern = NULL;
    CLEAN_BUFFER *nodes_buffer = NULL;
    
    nodes_buffer = mcp_params_parse_array_to_pattern(params, "nodes", false, false, MCP_TOOL_LIST_NODES, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer);
        return -1;
    }
    if (nodes_buffer) nodes_pattern = buffer_tostring(nodes_buffer);
    
    // Extract time parameters
    time_t after = mcp_params_parse_time(params, "after", MCP_DEFAULT_AFTER_TIME);
    time_t before = mcp_params_parse_time(params, "before", MCP_DEFAULT_BEFORE_TIME);
    
    // Extract cardinality limit
    size_t cardinality_limit = mcp_params_extract_size(params, "cardinality_limit", MCP_ALERTS_CARDINALITY_LIMIT, 1, MAX(MCP_ALERTS_CARDINALITY_LIMIT, MCP_ALERTS_CARDINALITY_LIMIT_MAX), error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer);
        return -1;
    }
    
    // Extract pagination cursor (global_id_anchor)
    usec_t global_id_anchor = 0;
    const char *cursor = mcp_params_extract_string(params, "cursor", NULL);
    if (cursor) {
        char *endptr;
        unsigned long long value = strtoull(cursor, &endptr, 10);
        if (*endptr != '\0' || endptr == cursor) {
            buffer_sprintf(error_buffer, "Invalid cursor value");
            mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
            buffer_free(error_buffer); buffer_free(nodes_buffer);
            return -1;
        }
        global_id_anchor = (usec_t)value;
    }
    
    // Extract status array (required parameter)
    CLEAN_BUFFER *status_buffer = NULL;
    const char *status_pattern = NULL;
    
    status_buffer = mcp_params_parse_array_to_pattern(params, "status", true, false, NULL, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        buffer_strcat(error_buffer, ". You must select at least one alert status to filter by.");
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer); buffer_free(status_buffer);
        return -1;
    }
    if (status_buffer) status_pattern = buffer_tostring(status_buffer);
    
    // Extract instances array
    const char *instances_pattern = NULL;
    CLEAN_BUFFER *instances_buffer = NULL;
    instances_buffer = mcp_params_parse_array_to_pattern(params, "instances", false, false, NULL, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer); buffer_free(status_buffer); buffer_free(instances_buffer);
        return -1;
    }
    if (instances_buffer) instances_pattern = buffer_tostring(instances_buffer);
    
    // Extract metrics array
    const char *metrics_pattern = NULL;
    CLEAN_BUFFER *metrics_buffer = NULL;
    metrics_buffer = mcp_params_parse_array_to_pattern(params, "metrics", false, false, MCP_TOOL_LIST_METRICS, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer); buffer_free(status_buffer); buffer_free(instances_buffer); buffer_free(metrics_buffer);
        return -1;
    }
    if (metrics_buffer) metrics_pattern = buffer_tostring(metrics_buffer);
    
    // Extract alerts array
    const char *alerts_pattern = NULL;
    CLEAN_BUFFER *alerts_buffer = NULL;
    alerts_buffer = mcp_params_parse_array_to_pattern(params, "alerts", false, false, MCP_TOOL_LIST_ALL_ALERTS, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer); buffer_free(status_buffer); buffer_free(instances_buffer); buffer_free(metrics_buffer); buffer_free(alerts_buffer);
        return -1;
    }
    if (alerts_buffer) alerts_pattern = buffer_tostring(alerts_buffer);
    
    // Extract classifications array
    const char *classifications_pattern = NULL;
    CLEAN_BUFFER *classifications_buffer = NULL;
    classifications_buffer = mcp_params_parse_array_to_pattern(params, "classifications", false, false, MCP_TOOL_LIST_ALL_ALERTS, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer); buffer_free(status_buffer); buffer_free(instances_buffer); buffer_free(metrics_buffer); buffer_free(alerts_buffer); buffer_free(classifications_buffer);
        return -1;
    }
    if (classifications_buffer) classifications_pattern = buffer_tostring(classifications_buffer);
    
    // Extract types array
    const char *types_pattern = NULL;
    CLEAN_BUFFER *types_buffer = NULL;
    types_buffer = mcp_params_parse_array_to_pattern(params, "types", false, false, MCP_TOOL_LIST_ALL_ALERTS, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer); buffer_free(status_buffer); buffer_free(instances_buffer); buffer_free(metrics_buffer); buffer_free(alerts_buffer); buffer_free(classifications_buffer); buffer_free(types_buffer);
        return -1;
    }
    if (types_buffer) types_pattern = buffer_tostring(types_buffer);
    
    // Extract components array
    const char *components_pattern = NULL;
    CLEAN_BUFFER *components_buffer = NULL;
    components_buffer = mcp_params_parse_array_to_pattern(params, "components", false, false, MCP_TOOL_LIST_ALL_ALERTS, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer); buffer_free(status_buffer); buffer_free(instances_buffer); buffer_free(metrics_buffer); buffer_free(alerts_buffer); buffer_free(classifications_buffer); buffer_free(types_buffer); buffer_free(components_buffer);
        return -1;
    }
    if (components_buffer) components_pattern = buffer_tostring(components_buffer);
    
    // Extract roles array
    const char *roles_pattern = NULL;
    CLEAN_BUFFER *roles_buffer = NULL;
    roles_buffer = mcp_params_parse_array_to_pattern(params, "roles", false, false, MCP_TOOL_LIST_ALL_ALERTS, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer); buffer_free(status_buffer); buffer_free(instances_buffer); buffer_free(metrics_buffer); buffer_free(alerts_buffer); buffer_free(classifications_buffer); buffer_free(types_buffer); buffer_free(components_buffer); buffer_free(roles_buffer);
        return -1;
    }
    if (roles_buffer) roles_pattern = buffer_tostring(roles_buffer);
    
    // Extract timeout parameter
    int timeout = mcp_params_extract_timeout(params, "timeout", 60, 1, 3600, error_buffer);
    if (buffer_strlen(error_buffer) > 0) {
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 400);
        buffer_free(error_buffer); buffer_free(nodes_buffer); buffer_free(status_buffer); buffer_free(instances_buffer); buffer_free(metrics_buffer); buffer_free(alerts_buffer); buffer_free(classifications_buffer); buffer_free(types_buffer); buffer_free(components_buffer); buffer_free(roles_buffer);
        return -1;
    }
    
    // Create request structure
    struct api_v2_contexts_request req = {
        .scope_nodes = nodes_pattern,
        .scope_contexts = NULL, // Not used for alert transitions
        .nodes = NULL,
        .contexts = NULL,
        .after = after,
        .before = before,
        .timeout_ms = timeout * 1000,  // Convert seconds to milliseconds
        .options = CONTEXTS_OPTION_CONFIGURATIONS | CONTEXTS_OPTION_MCP | CONTEXTS_OPTION_RFC3339 | CONTEXTS_OPTION_JSON_LONG_KEYS | CONTEXTS_OPTION_MINIFY,
        .cardinality_limit = cardinality_limit,
        .alerts = {
            .last = cardinality_limit,
            .global_id_anchor = global_id_anchor,
            .facets = {
                [ATF_STATUS] = status_pattern,
                [ATF_CLASS] = classifications_pattern,
                [ATF_TYPE] = types_pattern,
                [ATF_COMPONENT] = components_pattern,
                [ATF_ROLE] = roles_pattern,
                [ATF_NODE] = NULL, // nodes_pattern is used in scope_nodes
                [ATF_ALERT_NAME] = alerts_pattern,
                [ATF_CHART_NAME] = instances_pattern, // Historically, chart name was used for instance
                [ATF_CONTEXT] = metrics_pattern,
            },
        },
        .access_level = job->auth ? job->auth->access : HTTP_ACCESS_ANONYMOUS_DATA,
    };
    
    // Execute the query
    CONTEXTS_V2_MODE mode = CONTEXTS_V2_NODES | CONTEXTS_V2_ALERT_TRANSITIONS;
    CLEAN_BUFFER *t = buffer_create(0, NULL);
    int response_code = rrdcontext_to_json_v2(t, &req, mode);

    // Prepare for cleanup
    buffer_free(nodes_buffer);
    buffer_free(status_buffer);
    buffer_free(instances_buffer);
    buffer_free(metrics_buffer);
    buffer_free(alerts_buffer);
    buffer_free(classifications_buffer);
    buffer_free(types_buffer);
    buffer_free(components_buffer);
    buffer_free(roles_buffer);

    if (response_code != HTTP_RESP_OK) {
        buffer_sprintf(error_buffer, "Query failed with response code %d. Details: %s",
                       response_code, buffer_strlen(t) > 0 ? buffer_tostring(t) : "No details.");
        mcp_job_add_error_response(job, buffer_tostring(error_buffer), 500); // Or map response_code
        buffer_free(error_buffer);
        buffer_free(t);
        return -1;
    }
    
    // Wrap the result in the standard MCP content structure
    CLEAN_BUFFER *content_wrapper = buffer_create(0);
    buffer_json_initialize(content_wrapper, "\"", "\"", 0, true, BUFFER_JSON_OPTIONS_MINIFY);
    buffer_json_member_add_array(content_wrapper, "content");
    buffer_json_add_array_item_object(content_wrapper);
    buffer_json_member_add_string(content_wrapper, "type", "text");
    buffer_json_member_add_string(content_wrapper, "text", buffer_tostring(t));
    buffer_json_object_close(content_wrapper); // Close text content object
    buffer_json_array_close(content_wrapper);  // Close content array
    buffer_json_object_close(content_wrapper); // Close main result object
    buffer_json_finalize(content_wrapper);

    mcp_job_add_json_response(job, content_wrapper); // Takes ownership

    buffer_free(t);
    buffer_free(error_buffer);
    return 0;
}