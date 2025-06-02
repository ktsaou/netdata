#ifndef NETDATA_MCP_CORE_H
#define NETDATA_MCP_CORE_H

#include "mcp-job.h"
#include "mcp-response.h"
#include "libnetdata/buffer/buffer.h" // For BUFFER and HTTP_CONTENT_TYPE

// Forward declarations
struct mcp_req_job;
struct mcp_response_buffer; // Already defined in mcp-response.h, but good practice for clarity

// Core MCP execution function
int mcp_execute_tool(struct mcp_req_job *job);

// Response buffer management - MCP_REQ_JOB needs a `response_buffers` field for these.
// Assuming MCP_REQ_JOB will be extended with:
// struct mcp_response_buffer *response_buffers;
// And MCP_RESPONSE_BUFFER with:
// struct mcp_response_buffer *next;
// struct mcp_response_buffer *prev;
// And MCP_REQ_JOB with:
// int status_code;
// bool completed;
// uint64_t completed_usec;
// USER_AUTH *auth; // Added based on mcp-core.c usage
// const char *tool_name; // Added based on mcp-core.c usage (distinct from job->name which might be full path)


MCP_RESPONSE_BUFFER *mcp_job_add_response_buffer(struct mcp_req_job *job, BUFFER *buffer, const char *response_type);
void mcp_job_prepend_response_buffer(struct mcp_req_job *job, struct mcp_response_buffer *item);
void mcp_job_append_response_buffer(struct mcp_req_job *job, struct mcp_response_buffer *item);

// Helper functions for common response types
MCP_RESPONSE_BUFFER *mcp_job_add_text_response(struct mcp_req_job *job, const char *text, int http_status, HTTP_CONTENT_TYPE content_type);
MCP_RESPONSE_BUFFER *mcp_job_add_json_response(struct mcp_req_job *job, BUFFER *json_buffer); // Assumes json_buffer is already formatted JSON
MCP_RESPONSE_BUFFER *mcp_job_add_error_response(struct mcp_req_job *job, const char *error_msg, int http_status);

#endif // NETDATA_MCP_CORE_H
