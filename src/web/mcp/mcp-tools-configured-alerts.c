// SPDX-License-Identifier: GPL-3.0-or-later

#include "mcp-tools-configured-alerts.h"
#include "mcp-params.h"
#include "health/health_internals.h"

// Schema for list_configured_alerts - no parameters
void mcp_tool_list_configured_alerts_schema(BUFFER *buffer) {
    // Tool metadata
    buffer_json_member_add_object(buffer, "inputSchema");
    buffer_json_member_add_string(buffer, "type", "object");
    buffer_json_member_add_string(buffer, "title", "List configured alerts");
    
    buffer_json_member_add_object(buffer, "properties");
    // No properties - this tool accepts no parameters
    buffer_json_object_close(buffer); // properties
    
    buffer_json_object_close(buffer); // inputSchema
}

#include "core/mcp-core.h" // For mcp_job_add_error_response, mcp_job_add_json_response
#include "core/mcp-job.h"  // For MCP_REQ_JOB

// Execute list_configured_alerts - no filtering, returns all prototypes
int mcp_tool_list_configured_alerts(MCP_REQ_JOB *job) { // MCP_RETURN_CODE mcp_tool_list_configured_alerts_execute(MCP_CLIENT *mcpc, struct json_object *params __maybe_unused, MCP_REQUEST_ID id) {
    // This tool does not use job->params or job->auth directly, as it lists global config.
    if (!job) {
        return -1; // Should be caught by MCP core
    }
    
    // Create a temporary buffer for the result
    CLEAN_BUFFER *t = buffer_create(0, NULL);
    if (!t) {
        mcp_job_add_error_response(job, "Failed to allocate buffer for configured alerts list.", 500);
        return -1;
    }
    size_t count = 0;

    // Build the JSON response in tabular format
    buffer_json_initialize(t, "\"", "\"", 0, true, BUFFER_JSON_OPTIONS_MINIFY);
    buffer_json_member_add_uint64(t, "format_version", 1);
    
    // Add header for tabular data
    buffer_json_member_add_array(t, "configured_alerts_header");
    {
        buffer_json_add_array_item_string(t, "name");
        buffer_json_add_array_item_string(t, "applies_to");
        buffer_json_add_array_item_string(t, "on");
        buffer_json_add_array_item_string(t, "summary");
        buffer_json_add_array_item_string(t, "component");
        buffer_json_add_array_item_string(t, "classification");
        buffer_json_add_array_item_string(t, "type");
        buffer_json_add_array_item_string(t, "recipient");
    }
    buffer_json_array_close(t); // configured_alerts_header
    
    // Add tabular data
    buffer_json_member_add_array(t, "configured_alerts");
    {
        // Iterate through all alert prototypes
        RRD_ALERT_PROTOTYPE *ap;
        dfe_start_read(health_globals.prototypes.dict, ap) {
            // Only use the first rule for each prototype
            if (ap) {
                buffer_json_add_array_item_array(t); // Start row
                {
                    // name
                    buffer_json_add_array_item_string(t, string2str(ap->config.name));
                    
                    // alert_type (template or alarm)
                    buffer_json_add_array_item_string(t, ap->match.is_template ? "context" : "instance");
                    
                    // on (context or instance)
                    if(ap->match.is_template)
                        buffer_json_add_array_item_string(t, string2str(ap->match.on.context));
                    else
                        buffer_json_add_array_item_string(t, string2str(ap->match.on.chart));
                    
                    // summary
                    buffer_json_add_array_item_string(t, string2str(ap->config.summary));
                    
                    // component
                    buffer_json_add_array_item_string(t, string2str(ap->config.component));
                    
                    // classification
                    buffer_json_add_array_item_string(t, string2str(ap->config.classification));
                    
                    // type
                    buffer_json_add_array_item_string(t, string2str(ap->config.type));
                    
                    // recipient
                    buffer_json_add_array_item_string(t, string2str(ap->config.recipient));
                }
                buffer_json_array_close(t); // End row
                
                count++;
            }
        }
        dfe_done(ap);
    }
    buffer_json_array_close(t); // configured_alerts
    
    // Add summary
    buffer_json_member_add_object(t, "summary");
    {
        buffer_json_member_add_uint64(t, "total_prototypes", count);
    }
    buffer_json_object_close(t); // summary
    
    buffer_json_finalize(t);
    
    // Wrap the result in the standard MCP content structure
    CLEAN_BUFFER *content_wrapper = buffer_create(0);
    if (!content_wrapper) {
        mcp_job_add_error_response(job, "Failed to allocate content wrapper for configured alerts list.", 500);
        buffer_free(t);
        return -1;
    }
    buffer_json_initialize(content_wrapper, "\"", "\"", 0, true, BUFFER_JSON_OPTIONS_MINIFY);
    buffer_json_member_add_array(content_wrapper, "content");
    buffer_json_add_array_item_object(content_wrapper);
    buffer_json_member_add_string(content_wrapper, "type", "text");
    buffer_json_member_add_string(content_wrapper, "text", buffer_tostring(t));
    buffer_json_object_close(content_wrapper); // Close text content object
    buffer_json_array_close(content_wrapper);  // Close content array
    buffer_json_object_close(content_wrapper); // Close main result object
    buffer_json_finalize(content_wrapper);
    
    mcp_job_add_json_response(job, content_wrapper); // Takes ownership of content_wrapper

    buffer_free(t);
    return 0; // Success
}