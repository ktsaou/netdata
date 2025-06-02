// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_MCP_TOOLS_EXECUTE_FUNCTION_H
#define NETDATA_MCP_TOOLS_EXECUTE_FUNCTION_H

#include "mcp-tools.h" // For MCP_RETURN_CODE, MCP_REQUEST_ID if they are used internally or by schema
#include "core/mcp-job.h" // For MCP_REQ_JOB

// Schema definition function - provides JSON schema for this tool
void mcp_tool_execute_function_schema(BUFFER *buffer);

// Execution function - performs the function execution
int mcp_tool_execute_function(MCP_REQ_JOB *job);

#endif // NETDATA_MCP_TOOLS_EXECUTE_FUNCTION_H