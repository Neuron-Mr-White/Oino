/**
 * Settings › MCP (WP-A25, G-8): the `mcp` namespace's form (servers, catalog
 * source, tool filters) and the extension's own panels — status, server list
 * with the JSON editor, catalog add, sync and reload — which open in the
 * active chat through `/mcp-status`, `/mcp-settings`, `/mcp-add`, `/mcp-sync`
 * and `/mcp-reload`. No second server list lives here (§4.2.2).
 */
import { Plug } from 'lucide-solid';
import { CapabilityPage } from './CapabilityPage';

export function McpPage() {
  return (
    <CapabilityPage
      page="mcp"
      title="MCP"
      icon={Plug}
      namespace="mcp"
      sectionId="sec-mcp"
      blurb="Model Context Protocol servers: what the agent can call beyond its own tools."
    />
  );
}
