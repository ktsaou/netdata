// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_MCP_TOOLS_METRICS_QUERY_H
#define NETDATA_MCP_TOOLS_METRICS_QUERY_H

#include "mcp-tools.h"    // For MCP_RETURN_CODE (if used internally)
#include "core/mcp-job.h" // For MCP_REQ_JOB

// Schema definition function - provides JSON schema for this tool
void mcp_tool_query_metrics_schema(BUFFER *buffer);

// Execution function - performs the metrics query
int mcp_tool_query_metrics(MCP_REQ_JOB *job);

#endif // NETDATA_MCP_TOOLS_METRICS_QUERY_H