import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'pr402 Docs',
  description: 'x402 facilitator for Solana — HTTP 402 settled on-chain via UniversalSettle (exact) and SLA-Escrow.',
  // Explicit .html URLs: avoids hosts mishandling extensionless cleanUrls (nginx-style try_files).
  cleanUrls: false,
  trailingSlash: false,

  themeConfig: {
    logo: '/pr402.png',

    nav: [
      { text: 'Home', link: '/' },
      { text: 'Start here', link: '/start-here' },
      { text: 'Integrate (sellers)', link: '/seller-quick-start' },
      { text: 'Buyer Quickstart', link: '/quickstart-buyer' },
      { text: 'Why pr402?', link: '/pr402-vs-alternatives' },
      { text: 'Agent Integration', link: '/agent-integration' },
      { text: 'API Reference', link: '/api-reference' },
      {
        text: 'OpenAPI JSON',
        link: 'https://ipay.sh/openapi.json',
        target: '_blank',
        rel: 'noopener noreferrer',
      },
    ],

    sidebar: [
      {
        text: 'For sellers',
        items: [
          { text: 'Start here', link: '/start-here' },
          { text: 'Integrate your API', link: '/seller-quick-start' },
          { text: 'Quick reference · 5 steps', link: '/quickstart-seller' },
        ],
      },
      {
        text: 'For buyers',
        items: [
          { text: 'Buyer Quickstart', link: '/quickstart-buyer' },
        ],
      },
      {
        text: 'Choosing & policy',
        items: [
          { text: 'Choosing x402 on Solana', link: '/pr402-vs-alternatives' },
          { text: 'Onboarding Guide', link: '/onboarding_guide' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'API overview (humans + agents)', link: '/api-reference' },
        ],
      },
      {
        text: 'Deep Dives',
        items: [
          { text: 'Agent integration runbook', link: '/agent-integration' },
        ],
      },
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/miraland-labs/x402' },
    ],

    footer: {
      message: 'Built for the autonomous future.',
      copyright: 'Copyright © 2026 Miraland Labs',
    },

    search: {
      provider: 'local',
    },
  },
})
