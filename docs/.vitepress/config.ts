import { defineConfig } from "vitepress";

export default defineConfig({
  title: "Fresh",
  description:
    "Fresh is a fast, modern terminal text editor with intuitive keybindings, syntax highlighting, and instant startup.",
  base: "/fresh/docs/",
  srcDir: ".",
  outDir: "../dist/docs",

  head: [["link", { rel: "icon", href: "/fresh/docs/logo.svg" }]],

  cleanUrls: true,
  lastUpdated: true,
  appearance: "force-dark",
  themeConfig: {
    logo: { light: "/logo.svg", dark: "/logo.svg" },

    nav: [
      { text: "Homepage", link: "https://sinelaw.github.io/fresh" },
      { text: "Getting Started", link: "/index" },
      { text: "Download", link: "https://github.com/sinelaw/fresh/releases/latest" },
      {
        text: "Issues & Requests",
        link: "https://github.com/sinelaw/fresh/issues",
      },
    ],

    sidebar: [],

    outline: { level: "deep" },

    socialLinks: [{ icon: "github", link: "https://github.com/sinelaw/fresh" }],

    search: { provider: "local" },

    editLink: {
      pattern: "https://github.com/sinelaw/fresh/edit/master/docs/:path",
    },

    footer: {
      message: "Released under the Apache 2.0 License",
    },
  },
});
