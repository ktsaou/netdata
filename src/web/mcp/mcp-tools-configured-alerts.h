// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_MCP_TOOLS_CONFIGURED_ALERTS_H
#define NETDATA_MCP_TOOLS_CONFIGURED_ALERTS_H

#include "mcp-tools.h"    // For MCP_RETURN_CODE (if used internally)
#include "core/mcp-job.h" // For MCP_REQ_JOB

#define MCP_TOOL_LIST_CONFIGURED_ALERTS "list_configured_alerts"

// Schema generator function
void mcp_tool_list_configured_alerts_schema(BUFFER *buffer);

// Execution function
int mcp_tool_list_configured_alerts(MCP_REQ_JOB *job);

#endif //NETDATA_MCP_TOOLS_CONFIGURED_ALERTS_H