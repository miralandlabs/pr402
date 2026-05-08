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
      { text: 'API Reference', link: 'https://ipay.sh/openapi.json' } // API link back to raw or landing
    ],

    // Sidebar navigation
    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Seller Quickstart', link: '/seller-quick-start' },
          { text: 'Buyer Quickstart', link: '/quickstart-buyer' },
          { text: 'Onboarding Guide', link: '/onboarding_guide' }
        ]
      },
      {
        text: 'Deep Dives',
        items: [
          { text: 'Full Agent Integration', link: '/agent-integration' }
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
