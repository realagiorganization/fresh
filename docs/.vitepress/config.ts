import { defineConfig } from "vitepress";

export default defineConfig({
  title: "Fresh",
  description:
    "Fresh is a fast, modern terminal text editor with intuitive keybindings, syntax highlighting, and instant startup.",
  base: "/fresh/docs/",
  srcDir: ".",
  outDir: "../dist/docs",

  head: [["link", { rel: "icon", href: "/fresh/favicon.ico" }]],

  cleanUrls: true,
  lastUpdated: true,
  ignoreDeadLinks: ["/locales"],
  appearance: "force-dark",
  themeConfig: {
    logo: { light: "/logo.svg", dark: "/logo.svg" },

    nav: [
      { text: "Homepage", link: "https://radiorambo.github.io/fresh" },
      { text: "Getting Started", link: "/index" },
      { text: "Download", link: "https://github.com/sinelaw/fresh/releases/latest" },
      {
        text: "Issues & Requests",
        link: "https://github.com/sinelaw/fresh/issues",
      },
    ],

    sidebar: [
      {
        items: [{ text: "Getting Started", link: "/index" },{text:"internal", link: "/internal"}],
      },
      {
        text: "User Guide",
        collapsed: false,
        items: [
          { text: "Introduction", link: "/guide/" },
          { text: "Editing & Navigation", link: "/guide/editing" },
          { text: "Terminal", link: "/guide/terminal" },
          { text: "LSP Integration", link: "/guide/lsp" },
          { text: "Plugins", link: "/guide/plugins" },
          { text: "Themes", link: "/guide/themes" },
          { text: "Configuration", link: "/guide/configuration" },
          { text: "Keyboard Setup", link: "/guide/keyboard" },
          { text: "Internationalization", link: "/guide/i18n" },
          { text: "Troubleshooting", link: "/guide/troubleshooting" },
          { text: "Keybindings", link: "/guide/keybindings" },
        ],
      },
      {
        text: "Features",
        collapsed: false,
        items: [
          { text: "Terminal", link: "/features/terminal" },
          { text: "Vi Mode", link: "/features/vi-mode" },
        ],
      },
      {
        text: "Development",
        collapsed: false,
        items: [
          { text: "Architecture", link: "/development/architecture" },
          { text: "Plugin API", link: "/development/plugin-api" },
          { text: "Plugin Development", link: "/development/plugin-development" },
        ],
      },
      {
        text: "Design Documents",
        collapsed: true,
        items: [
          { text: "Config Editor", link: "/design/config-editor" },
          { text: "Paste Handling", link: "/design/paste-handling" },
          { text: "Scroll Sync", link: "/design/scroll-sync" },
          { text: "Unicode Width", link: "/design/unicode-width" },
          { text: "Visual Layout", link: "/design/visual-layout" },
          { text: "Internationalization", link: "/design/i18n" },
          { text: "Search Next Occurrence", link: "/design/search-next-occurrence" },
        ],
      },
    ],

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
