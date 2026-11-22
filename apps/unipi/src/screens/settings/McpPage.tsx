/** Settings › MCP: one native scope-aware server list and Add/Edit/catalog sheet. */
import { Plug } from 'lucide-solid';
import { Section } from './rows';
import { McpServers } from '../../components/mcp/McpServers';
import { settingsHostId } from '../../settings/target';

export function McpPage() {
  const hostId = () => settingsHostId();
  return (
    <Section title="MCP" id="sec-mcp" icon={<Plug size={14} />} intro="Model Context Protocol servers. Global servers apply everywhere; project servers belong to the selected folder.">
      <McpServers hostId={hostId()} />
    </Section>
  );
}
