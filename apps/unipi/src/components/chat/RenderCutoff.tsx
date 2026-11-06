import './renderCutoff.css';

/** The model's inclusive context anchor, not a deletion of the human transcript. */
export function RenderCutoff() {
  return <div class="render-cutoff" role="separator" aria-label="Model context starts here" data-testid="render-cutoff">
    <span /> <span>Model context starts here · earlier history preserved</span> <span />
  </div>;
}
