// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_MCP_TOOLS_WEIGHTS_H
#define NETDATA_MCP_TOOLS_WEIGHTS_H

#include "mcp.h" // For MCP_RETURN_CODE (if used internally)
#include "core/mcp-job.h" // For MCP_REQ_JOB

// Execute the find_correlated_metrics tool
int mcp_tool_find_correlated_metrics(MCP_REQ_JOB *job);
void mcp_tool_find_correlated_metrics_schema(BUFFER *buffer);

// Execute the find_anomalous_metrics tool
int mcp_tool_find_anomalous_metrics(MCP_REQ_JOB *job);
void mcp_tool_find_anomalous_metrics_schema(BUFFER *buffer);

// Execute the find_unstable_metrics tool
int mcp_tool_find_unstable_metrics(MCP_REQ_JOB *job);
void mcp_tool_find_unstable_metrics_schema(BUFFER *buffer);

#endif // NETDATA_MCP_TOOLS_WEIGHTS_H