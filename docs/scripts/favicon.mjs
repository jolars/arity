export const faviconHead = [
  [
    "link",
    {
      rel: "icon",
      type: "image/png",
      href: "/favicon-96x96.png",
      sizes: "96x96",
    },
  ],
  ["link", { rel: "icon", href: "/favicon.svg", type: "image/svg+xml" }],
  ["link", { rel: "shortcut icon", href: "/favicon.ico" }],
  [
    "link",
    {
      rel: "apple-touch-icon",
      sizes: "180x180",
      href: "/apple-touch-icon.png",
    },
  ],
  ["meta", { name: "apple-mobile-web-app-title", content: "Arity" }],
  ["link", { rel: "manifest", href: "/site.webmanifest" }],
];
