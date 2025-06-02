// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_MCP_TOOLS_ALERT_TRANSITIONS_H
#define NETDATA_MCP_TOOLS_ALERT_TRANSITIONS_H

#include "mcp.h" // For MCP_RETURN_CODE (if used internally)
#include "core/mcp-job.h" // For MCP_REQ_JOB

// Function declarations for alert transitions tool
void mcp_tool_list_alert_transitions_schema(BUFFER *buffer);
int mcp_tool_list_alert_transitions(MCP_REQ_JOB *job);

#endif //NETDATA_MCP_TOOLS_ALERT_TRANSITIONS_H