import { defineConfig } from 'vitepress'

// https://vitepress.dev/reference/site-config
export default defineConfig({
  title: "Night Amplifier",
  description: "User Manual for Night Amplifier",
  base: '/night-amplifier/', // For github pages
  themeConfig: {
    // https://vitepress.dev/reference/default-theme-config
    nav: [
      { text: 'Home', link: '/' },
      { text: 'Getting Started', link: '/getting-started' }
    ],

    sidebar: [
      {
        text: 'User Guide',
        items: [
          { text: 'Getting Started', link: '/getting-started' },
          { text: 'Live Stacking', link: '/stacking' },
          { text: 'Sensor Corrections', link: '/sensor-corrections' }
        ]
      }
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/Nikita-Kudrin/night-amplifier' }
    ]
  }
})
