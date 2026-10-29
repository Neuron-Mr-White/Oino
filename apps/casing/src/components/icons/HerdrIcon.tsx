/**
 * herdr sheep-head icon in the Moshi style: a scalloped wool outline
 * filled with currentColor, a darker face oval cut out low-centre and
 * two small ears. Reads as a sheep down to ~14 px.
 */
export function HerdrIcon(props: { size?: number }) {
  const size = () => props.size ?? 18;
  return (
    <svg
      width={size()}
      height={size()}
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
    >
      <mask id="herdr-sheep-face">
        <rect width="24" height="24" fill="white" />
        {/* face oval, cut out low-centre */}
        <ellipse cx="12" cy="14.6" rx="3.7" ry="3.1" fill="black" />
      </mask>
      <g mask="url(#herdr-sheep-face)" fill="currentColor">
        {/* ears */}
        <ellipse cx="3.9" cy="10.6" rx="2.1" ry="1.35" transform="rotate(-24 3.9 10.6)" />
        <ellipse cx="20.1" cy="10.6" rx="2.1" ry="1.35" transform="rotate(24 20.1 10.6)" />
        {/* scalloped wool: overlapping lobes read as one fluffy head */}
        <circle cx="7.6" cy="9.4" r="4.1" />
        <circle cx="12" cy="7.6" r="4.6" />
        <circle cx="16.4" cy="9.4" r="4.1" />
        <circle cx="5.9" cy="13.6" r="3.5" />
        <circle cx="18.1" cy="13.6" r="3.5" />
        <circle cx="9" cy="15.9" r="3.2" />
        <circle cx="15" cy="15.9" r="3.2" />
        <circle cx="12" cy="16.6" r="3.4" />
      </g>
    </svg>
  );
}
