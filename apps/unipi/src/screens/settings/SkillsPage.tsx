/**
 * Settings › Skills (WP-A25, G-10): the `skills` namespace (exposure mode,
 * pins, the vault) rendered by the form engine, plus the skills hub and the
 * per-skill editor panels (`/skills`, `/skills-editor`).
 */
import { Sparkles } from 'lucide-solid';
import { CapabilityPage } from './CapabilityPage';

export function SkillsPage() {
  return (
    <CapabilityPage
      page="skills"
      title="Skills"
      icon={Sparkles}
      namespace="skills"
      sectionId="sec-skills"
      blurb="Which skills the agent sees, which it must use, and what stays in the vault."
    />
  );
}
