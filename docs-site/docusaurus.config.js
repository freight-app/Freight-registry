// @ts-check

const config = {
  title: 'Freight Ecosystem',
  tagline: 'Build, publish, document, and consume native packages with Freight.',
  favicon: 'img/favicon.ico',
  url: 'https://freight.dev',
  baseUrl: '/docs/',
  organizationName: 'freight-app',
  projectName: 'freight',
  trailingSlash: true,

  onBrokenLinks: 'throw',
  onBrokenMarkdownLinks: 'warn',

  presets: [
    [
      'classic',
      {
        docs: {
          routeBasePath: '/',
          sidebarPath: './sidebars.js',
          editUrl: 'https://github.com/freight-app/freight-registry/tree/main/docs-site/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      },
    ],
  ],

  themeConfig: {
    colorMode: {
      defaultMode: 'dark',
      disableSwitch: false,
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'Freight',
      items: [
        { type: 'docSidebar', sidebarId: 'guideSidebar', position: 'left', label: 'Guide' },
        { to: '/install/', label: 'Install', position: 'left' },
        { href: '/', label: 'Registry', position: 'right' },
        { href: 'https://github.com/freight-app/freight-registry', label: 'GitHub', position: 'right' },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Docs',
          items: [
            { label: 'Guide', to: '/' },
            { label: 'Install', to: '/install/' },
            { label: 'Publish packages', to: '/publish/' },
          ],
        },
        {
          title: 'Registry',
          items: [
            { label: 'Package search', href: '/' },
            { label: 'Status', href: '/health' },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Freight.`,
    },
    prism: {
      additionalLanguages: ['toml', 'powershell', 'bash'],
    },
  },
};

module.exports = config;
