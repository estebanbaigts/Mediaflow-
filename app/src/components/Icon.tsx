// A small, consistent icon set: 16×16 grid, 1.5 stroke, round caps.
import type { SVGProps } from "react";

const paths: Record<string, string> = {
  overview: "M2.5 2.5h4.5v5H2.5zM9 2.5h4.5v3H9zM9 7.5h4.5v6H9zM2.5 9.5h4.5v4H2.5z",
  import: "M8 2v8m0 0L5 7m3 3 3-3M2.5 11v1.5a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1V11",
  projects: "M2 4.5a1 1 0 0 1 1-1h3l1.5 1.5H13a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z",
  library: "M3 2.5h2.5v11H3zM7 2.5h2.5v11H7zM11 3l2.3-.6 2 10.6-2.3.5z",
  duplicates: "M5.5 5.5h8v8h-8zM2.5 10.5v-8h8",
  verify: "M8 1.8 13.5 4v4c0 3-2.3 5.3-5.5 6.2C4.8 13.3 2.5 11 2.5 8V4zM5.5 8l1.8 1.8 3.4-3.6",
  history: "M2.5 8a5.5 5.5 0 1 0 1.6-3.9M2.5 2.5v2.2h2.2M8 5v3.2l2.2 1.3",
  settings:
    "M8 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM13 8.8V7.2l-1.6-.4-.5-1.2.9-1.4-1.1-1.1-1.4.9-1.2-.5L7.8 2H6.2l-.4 1.5-1.2.5-1.4-.9-1.1 1.1.9 1.4-.5 1.2L1 7.2v1.6l1.5.4.5 1.2-.9 1.4 1.1 1.1 1.4-.9 1.2.5.4 1.5h1.6l.4-1.5 1.2-.5 1.4.9 1.1-1.1-.9-1.4.5-1.2z",
  folder: "M2 4.5a1 1 0 0 1 1-1h3l1.5 1.5H13a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z",
  drive: "M2 9.5 3.6 4a1 1 0 0 1 1-.7h6.8a1 1 0 0 1 1 .7L14 9.5M2 9.5h12v3a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1zM11 11.5h.01",
  card: "M4 1.8h5.5L12.5 5v9a.8.8 0 0 1-.8.8H4.3a.8.8 0 0 1-.8-.8V2.6a.8.8 0 0 1 .5-.8zM6 3.8v1.6M8 3.8v1.6M10 4.5v.9",
  check: "M3 8.5 6.5 12 13 4.5",
  warn: "M8 2 14.5 13.5h-13zM8 6.5v3M8 11.5h.01",
  error: "M8 14.5a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13zM5.8 5.8l4.4 4.4M10.2 5.8l-4.4 4.4",
  info: "M8 14.5a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13zM8 7.2V11M8 5h.01",
  pause: "M5.5 3.5v9M10.5 3.5v9",
  play: "M5 3.2v9.6L12.5 8z",
  stop: "M4 4h8v8H4z",
  plus: "M8 3v10M3 8h10",
  x: "M4 4l8 8M12 4l-8 8",
  chevron: "M6 3.5 10.5 8 6 12.5",
  back: "M10 3.5 5.5 8l4.5 4.5",
  search: "M7 12a5 5 0 1 0 0-10 5 5 0 0 0 0 10zM10.6 10.6 14 14",
  export: "M8 10V2m0 0L5 5m3-3 3 3M3 9v4.5h10V9",
  copy: "M5.5 5.5h8v8h-8zM2.5 10.5v-8h8",
  skip: "M3 3.5 8 8l-5 4.5M9 3.5 14 8l-5 4.5",
  lock: "M4 7.5h8v6H4zM5.5 7.5V5a2.5 2.5 0 0 1 5 0v2.5",
  refresh: "M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2.5v2.2h-2.2",
};

export type IconName = keyof typeof paths;

export function Icon({ name, size = 16, ...rest }: { name: IconName; size?: number } & SVGProps<SVGSVGElement>) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      <path d={paths[name]} />
    </svg>
  );
}
