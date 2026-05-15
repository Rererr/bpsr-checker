import type { JSX } from "solid-js";

type IconProps = { size?: number };

export function TinaIcon(props: IconProps): JSX.Element {
  const s = props.size ?? 20;
  return (
    <svg width={s} height={s} viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg">
      <circle cx="10" cy="10" r="8" stroke="currentColor" stroke-width="1.5" />
      <circle cx="10" cy="10" r="1" fill="currentColor" />
      {/* minute hand */}
      <line x1="10" y1="10" x2="10" y2="4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      {/* hour hand */}
      <line x1="10" y1="10" x2="13" y2="7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      {/* tick marks */}
      <line x1="10" y1="2.5" x2="10" y2="3.5" stroke="currentColor" stroke-width="1" />
      <line x1="10" y1="16.5" x2="10" y2="17.5" stroke="currentColor" stroke-width="1" />
      <line x1="2.5" y1="10" x2="3.5" y2="10" stroke="currentColor" stroke-width="1" />
      <line x1="16.5" y1="10" x2="17.5" y2="10" stroke="currentColor" stroke-width="1" />
    </svg>
  );
}

export function AlunaIcon(props: IconProps): JSX.Element {
  const s = props.size ?? 20;
  return (
    <svg width={s} height={s} viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* wings */}
      <path d="M10 14 C6 12 2 9 3 5 C5 7 7 8 10 10" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none" />
      <path d="M10 14 C14 12 18 9 17 5 C15 7 13 8 10 10" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none" />
      {/* cross / revival */}
      <line x1="10" y1="3" x2="10" y2="10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
      <line x1="7.5" y1="5.5" x2="12.5" y2="5.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
      {/* halo */}
      <ellipse cx="10" cy="2.5" rx="3" ry="1" stroke="currentColor" stroke-width="1" fill="none" />
      {/* body */}
      <line x1="10" y1="10" x2="10" y2="17" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      <line x1="7" y1="12" x2="13" y2="12" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
    </svg>
  );
}

export function TartaIcon(props: IconProps): JSX.Element {
  const s = props.size ?? 20;
  return (
    <svg width={s} height={s} viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* shield shape */}
      <path
        d="M10 2 L17 5 L17 11 C17 15 13.5 17.5 10 18.5 C6.5 17.5 3 15 3 11 L3 5 Z"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-linejoin="round"
        fill="none"
      />
      {/* inner detail */}
      <path
        d="M10 5 L14 7 L14 11 C14 13.5 12 15 10 15.8 C8 15 6 13.5 6 11 L6 7 Z"
        stroke="currentColor"
        stroke-width="0.8"
        stroke-linejoin="round"
        fill="none"
        opacity="0.6"
      />
    </svg>
  );
}

export function BasiliskIcon(props: IconProps): JSX.Element {
  const s = props.size ?? 20;
  return (
    <svg width={s} height={s} viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* sword blade */}
      <line x1="10" y1="2" x2="10" y2="14" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      {/* crossguard */}
      <line x1="7" y1="11" x2="13" y2="11" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      {/* grip */}
      <line x1="10" y1="14" x2="10" y2="17" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
      {/* up arrow */}
      <polyline points="14,5 17,2 17,7" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" fill="none" />
      <line x1="17" y1="2" x2="14" y2="8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
    </svg>
  );
}
