// Inkue brand mark — a "K" whose left stem is an "i" whose tittle is a play
// triangle. Source artwork: docs/design/inkue-logo-app-icon-1024.svg.
// Gradient/clip IDs are made unique per instance so multiple marks can coexist.

import { useId } from "react";

export function InkueMark({ size = 20 }: { size?: number }) {
  const uid = useId();
  const bar = `${uid}-bar`;
  const tri = `${uid}-tri`;
  const clip = `${uid}-clip`;
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" fill="none" xmlns="http://www.w3.org/2000/svg" aria-label="Inkue">
      <defs>
        <linearGradient id={bar} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#52D48A" />
          <stop offset="100%" stopColor="#1F8F4A" />
        </linearGradient>
        <linearGradient id={tri} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" stopColor="#3DBA6F" />
          <stop offset="100%" stopColor="#A8F0C6" />
        </linearGradient>
        <clipPath id={clip}>
          <rect x="0" y="0" width="1024" height="1024" rx="204.8" />
        </clipPath>
      </defs>
      <rect x="0" y="0" width="1024" height="1024" rx="204.8" fill="#0d0d10" />
      <ellipse cx="512" cy="901.12" rx="327.68" ry="61.44" fill="#3DBA6F" opacity="0.08" clipPath={`url(#${clip})`} />
      <rect x="302" y="296" width="112" height="432" rx="20.16" fill={`url(#${bar})`} />
      <polygon points="438,296 438,728 726,512" fill={`url(#${tri})`} clipPath={`url(#${clip})`} />
    </svg>
  );
}
