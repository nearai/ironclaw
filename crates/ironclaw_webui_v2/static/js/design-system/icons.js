import { html } from "../lib/html.js";

const paths = {
  attach: html`<path
    d="m21.4 11.1-9.2 9.2a6 6 0 0 1-8.5-8.5l9.2-9.2a4 4 0 0 1 5.7 5.7l-9.2 9.2a2 2 0 0 1-2.8-2.8l8.5-8.5"
  />`,

  bolt: html`<path d="M13 2.8 5.8 13h5.1L10 21.2 18.2 10h-5.4L13 2.8Z" />`,

  bell: html`<path d="M6.8 10.3a5.2 5.2 0 0 1 10.4 0v3.1l1.8 3.1H5l1.8-3.1v-3.1Z" /><path d="M9.7 19a2.5 2.5 0 0 0 4.6 0" />`,

  bookOpen: html`<path d="M4.5 5.5h5.2a3.2 3.2 0 0 1 3.2 3.2v10.8a3.8 3.8 0 0 0-3.2-1.7H4.5V5.5Z" /><path d="M19.5 5.5h-5.2a3.2 3.2 0 0 0-3.2 3.2v10.8a3.8 3.8 0 0 1 3.2-1.7h5.2V5.5Z" /><path d="M7.5 9h2.2M7.5 12h2.2M16.3 9h.2M16.3 12h.2" />`,

  calendar: html`<path d="M6.5 4.5v3M17.5 4.5v3" /><path
      d="M4.5 7h15v12.5h-15V7Z"
    /><path d="M4.5 10.5h15" /><path d="M8 14h.1M12 14h.1M16 14h.1M8 17h.1M12 17h.1" />`,

  check: html`<path d="m5 12.5 4.3 4.3L19.2 6.7" />`,

  chat: html`<path d="M5 5.5h14v10H9.4L5 19.2V5.5Z" /><path
      d="M8.4 9h7.2M8.4 12.2h4.8"
    />`,

  close: html`<path d="m6.5 6.5 11 11M17.5 6.5l-11 11" />`,

  clock: html`<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17Z" /><path
      d="M12 7.5v5l3.2 2"
    />`,

  download: html`<path d="M12 3.8v10" /><path d="m8 10 4 4 4-4" /><path
      d="M5 17.5v2.7h14v-2.7"
    />`,

  file: html`<path d="M6.5 3.5h7.2L18 7.8v12.7H6.5v-17Z" /><path
      d="M13.7 3.5V8H18"
    />`,

  flag: html`<path d="M6.5 21V4.5" /><path d="M6.5 5h10.7l-1.4 4 1.4 4H6.5" />`,

  pin: html`<path d="M9 3.5h6l-1 5 3 3.5H7l3-3.5-1-5Z" /><path d="M12 15.5V21" />`,

  pause: html`<path d="M8.5 5.5v13" /><path d="M15.5 5.5v13" />`,

  play: html`<path d="M8 5.5 18.5 12 8 18.5V5.5Z" />`,

  folder: html`<path
    d="M3.5 7h6.2l1.9 2h8.9v9.2a2.3 2.3 0 0 1-2.3 2.3H5.8a2.3 2.3 0 0 1-2.3-2.3V7Z"
  />`,

  layers: html`<path d="m12 3.7 8.5 4.2-8.5 4.4-8.5-4.4L12 3.7Z" /><path
      d="m5.2 11.2 6.8 3.5 6.8-3.5"
    /><path d="m5.2 14.8 6.8 3.5 6.8-3.5" />`,

  list: html`<path d="M8.5 6.5h11M8.5 12h11M8.5 17.5h11" /><path
      d="M4.5 6.5h.1M4.5 12h.1M4.5 17.5h.1"
    />`,

  logs: html`<path d="M4.5 5.5h15v13h-15v-13Z" /><path
      d="m7.5 10 2 2-2 2"
    /><path d="M11.5 14h4.5" />`,

  lock: html`<path d="M7.5 10V7.2a4.5 4.5 0 0 1 9 0V10" /><path
      d="M5.5 10h13v10.5h-13V10Z"
    /><path d="M12 14.4v2.3" />`,

  logout: html`<path d="M10 17 15 12l-5-5" /><path d="M15 12H3.5" /><path
      d="M14.5 4.5H19a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2h-4.5"
    />`,

  moon: html`<path
    d="M20.2 14.7A7.7 7.7 0 0 1 9.3 3.8 8.4 8.4 0 1 0 20.2 14.7Z"
  />`,

  plug: html`<path d="M9 3.5v5M15 3.5v5" /><path
      d="M7.5 8.5h9v3.2a4.5 4.5 0 0 1-9 0V8.5Z"
    /><path d="M12 16.2v4.3" />`,

  plus: html`<path d="M12 5.5v13M5.5 12h13" />`,

  pulse: html`<path d="M3.5 12h4l2-5.5 4.2 11 2.2-5.5h4.6" />`,

  send: html`<path d="M4 11.8 20 4l-4.8 16-3.2-6.8L4 11.8Z" /><path
      d="m12 13.2 4.5-4.6"
    />`,

  search: html`<path d="M10.8 5.2a5.6 5.6 0 1 1 0 11.2 5.6 5.6 0 0 1 0-11.2Z" /><path
      d="m15.1 15.1 4 4"
    />`,

  settings: html`
    <path
      d="m19.14 12.94 2.06-1.44-1.73-3-2.47 1a7.07 7.07 0 0 0-1.47-.86L15.12 6h-3.46l-.42 2.64a7.07 7.07 0 0 0-1.47.86l-2.47-1-1.73 3 2.06 1.44a7.1 7.1 0 0 0 0 1.72l-2.06 1.44 1.73 3 2.47-1a7.07 7.07 0 0 0 1.47.86l.42 2.64h3.46l.42-2.64a7.07 7.07 0 0 0 1.47-.86l2.47 1 1.73-3-2.06-1.44a7.1 7.1 0 0 0 0-1.72Z"
    />`,

  spark: html`<path
    d="M12 3.5 14 10l6.5 2-6.5 2-2 6.5-2-6.5-6.5-2 6.5-2 2-6.5Z"
  />`,

  sun: html`<path d="M12 7.6a4.4 4.4 0 1 1 0 8.8 4.4 4.4 0 0 1 0-8.8Z" /><path
      d="M12 2.8v2.2M12 19v2.2M4.9 4.9l1.6 1.6M17.5 17.5l1.6 1.6M2.8 12H5M19 12h2.2M4.9 19.1l1.6-1.6M17.5 6.5l1.6-1.6"
    />`,

  shield: html`<path
      d="M12 3.2 4 7.1v4.5c0 4.7 3.3 8.9 8 10.2 4.7-1.3 8-5.5 8-10.2V7.1l-8-3.9Z"
    /><path d="m9.3 12 2 2 3.8-3.8" />`,

  tool: html`<path
    d="M15.3 4.4a4.5 4.5 0 0 0-5.7 5.7L4.8 15a2.7 2.7 0 1 0 3.8 3.8l4.9-4.8a4.5 4.5 0 0 0 5.7-5.7l-3.3 3.3-3.2-3.2 2.6-4Z"
  />`,

  terminal: html`<path d="M4.5 5.5h15v13h-15v-13Z" /><path d="m7.5 9.2 2.4 2.3-2.4 2.3" /><path d="M11.8 14h4.7" />`,

  trash: html`<path d="M5.5 7h13" /><path d="M9.5 7V4.5h5V7" /><path
      d="M7.2 7 8 20h8l.8-13"
    /><path d="M10.5 10.5v6M13.5 10.5v6" />`,

  upload: html`<path d="M12 14.2v-10" /><path d="m8 8.2 4-4 4 4" /><path
      d="M5 17.5v2.7h14v-2.7"
    />`,

  chevron: html`<path d="m6 9 6 6 6-6" />`,

  more: html`<path d="M12 5.6h.01M12 12h.01M12 18.4h.01" />`,

  copy: html`<path d="M9 9h9a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1Z" /><path
      d="M5 15a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1"
    />`,

  arrowDown: html`<path d="M12 5v14" /><path d="m6 13 6 6 6-6" />`,

  retry: html`<path d="M3.5 12a8.5 8.5 0 1 1 2.6 6.1" /><path d="M3.2 18.5v-5h5" />`,
};

export function Icon({ name, className = "", strokeWidth = 1.7 }) {
  return html`
    <svg
      aria-hidden="true"
      className=${className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth=${String(strokeWidth)}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      ${paths[name] || paths.spark}
    </svg>
  `;
}
