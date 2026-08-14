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
      { text: 'Start here', link: '/start-here' },
      {
        text: 'Sellers',
        items: [
          { text: 'Integrate your API', link: '/seller-quick-start' },
          { text: 'Hands-on lab', link: '/seller-lab' },
          { text: 'Quick reference', link: '/quickstart-seller' },
        ],
      },
      {
        text: 'Buyers',
        items: [
          { text: 'Buyer quickstart', link: '/quickstart-buyer' },
          { text: 'Connect Cursor', link: '/connect-cursor-to-pr402' },
        ],
      },
      { text: 'Why pr402?', link: '/pr402-vs-alternatives' },
      {
        text: 'Reference',
        items: [
          { text: 'API overview', link: '/api-reference' },
          { text: 'Agent integration', link: '/agent-integration' },
          { text: 'Resource discovery', link: '/discovery' },
          {
            text: 'OpenAPI JSON',
            link: 'https://ipay.sh/openapi.json',
            target: '_blank',
            rel: 'noopener noreferrer',
          },
        ],
      },
    ],

    sidebar: [
      {
        text: 'For sellers',
        items: [
          { text: 'Start here', link: '/start-here' },
          { text: 'Hands-on seller lab', link: '/seller-lab' },
          { text: 'Integrate your API', link: '/seller-quick-start' },
          { text: 'Quick reference · 5 steps', link: '/quickstart-seller' },
        ],
      },
      {
        text: 'For buyers',
        items: [
          { text: 'Buyer Quickstart', link: '/quickstart-buyer' },
          { text: 'Connect Cursor to pr402', link: '/connect-cursor-to-pr402' },
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
          { text: 'Resource discovery', link: '/discovery' },
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
      { icon: 'discord', link: 'https://discord.gg/VmBfyeM4YB' },
      { icon: 'x', link: 'https://x.com/hashspace_mi' },
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
