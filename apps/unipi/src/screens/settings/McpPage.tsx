/**
 * Settings › MCP (WP-A25, G-8; owner 2026-11-05): the native server table (status,
 * full errors, enable / reconnect per server, reload all, sync) and the two panels that
 * still need the extension's own UI — "Manage servers" (edit / remove JSON) and "Add from
 * the catalog". The form's four action rows duplicated the Manage rows and are hidden.
 */
import { Plug } from 'lucide-solid';
import { CapabilityPage } from './CapabilityPage';
import { McpServers } from '../../components/mcp/McpServers';
import { settingsHostId } from '../../settings/target';

export function McpPage() {
  const hostId = () => settingsHostId();
  return (
    <CapabilityPage
      page="mcp"
      title="MCP"
      icon={Plug}
      namespace="mcp"
      sectionId="sec-mcp"
      blurb="Model Context Protocol servers: what the agent can call beyond its own tools."
      hideFields={['configure', 'add', 'sync', 'reload']}
      beforeManage={<McpServers hostId={hostId()} />}
    />
  );
}
