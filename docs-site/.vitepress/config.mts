import { defineConfig } from 'vitepress'

export default defineConfig({
  title: "pr402 Docs",
  description: "The Liquidity Layer for the Autonomous Web",
  // Explicit .html URLs: avoids hosts mishandling extensionless cleanUrls (nginx-style try_files).
  cleanUrls: false,
  trailingSlash: false,
  
  // Theme related configurations
  themeConfig: {
    logo: '/pr402.png',
    
    // Navigation bar at the top
    nav: [
      { text: 'Home', link: '/' },
      { text: 'Seller Quickstart', link: '/seller-quick-start' },
      { text: 'Buyer Quickstart', link: '/quickstart-buyer' },
      { text: 'API Reference', link: '/api-reference' },
      {
        text: 'OpenAPI JSON',
        link: 'https://ipay.sh/openapi.json',
        target: '_blank',
        rel: 'noopener noreferrer'
      }
    ],

    // Sidebar navigation
    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Seller Quickstart', link: '/seller-quick-start' },
          { text: 'Seller shortcut (5 steps)', link: '/quickstart-seller' },
          { text: 'Buyer Quickstart', link: '/quickstart-buyer' },
          { text: 'Onboarding Guide', link: '/onboarding_guide' }
        ]
      },
      {
        text: 'Reference',
        items: [
          { text: 'API overview (humans + agents)', link: '/api-reference' }
        ]
      },
      {
        text: 'Deep Dives',
        items: [
          { text: 'Agent integration runbook', link: '/agent-integration' }
        ]
      }
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/miraland-labs/x402' }
    ],

    footer: {
      message: 'Built for the Autonomous Future.',
      copyright: 'Copyright © 2026 Miraland Labs'
    },
    
    // Enable local search
    search: {
      provider: 'local'
    }
  }
})
