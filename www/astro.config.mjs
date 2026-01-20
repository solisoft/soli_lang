// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

import tailwindcss from '@tailwindcss/vite';

// https://astro.build/config
export default defineConfig({
  integrations: [
      starlight({
          title: 'Solilang',
          description: 'A statically-typed, class-based OOP language with pipeline operators',
          social: [
              { icon: 'github', label: 'GitHub', href: 'https://github.com/solilang/solilang' },
          ],
          logo: {
              src: './src/assets/logo.svg',
          },
          customCss: ['./src/styles/custom.css'],
          sidebar: [
              {
                  label: 'Getting Started',
                  items: [
                      { label: 'Introduction', slug: 'guides/introduction' },
                      { label: 'Installation', slug: 'guides/installation' },
                      { label: 'Quick Start', slug: 'guides/quickstart' },
                  ],
              },
               {
                   label: 'Language Guide',
                   items: [
                       { label: 'Variables & Types', slug: 'guides/variables' },
                       { label: 'Functions', slug: 'guides/functions' },
                       { label: 'Control Flow', slug: 'guides/control-flow' },
                       { label: 'Arrays', slug: 'guides/arrays' },
                       { label: 'Hashes', slug: 'guides/hashes' },
                       { label: 'Classes & OOP', slug: 'guides/classes' },
                       { label: 'Date & Time', slug: 'guides/datetime' },
                       { label: 'Internationalization', slug: 'guides/internationalization' },
                       { label: 'Pipeline Operator', slug: 'guides/pipeline' },
                       { label: 'Modules & Packages', slug: 'guides/modules' },
                   ],
               },
              {
                  label: 'Reference',
                  autogenerate: { directory: 'reference' },
              },
              {
                  label: 'Internals',
                  items: [
                      { label: 'Execution Modes', slug: 'internals/execution-modes' },
                      { label: 'Bytecode VM', slug: 'internals/bytecode-vm' },
                      { label: 'JIT Compilation', slug: 'internals/jit-compilation' },
                  ],
              },
          ],
          head: [
              {
                  tag: 'meta',
                  attrs: {
                      property: 'og:image',
                      content: '/og-image.png',
                  },
              },
          ],
      }),
	],

  vite: {
    plugins: [tailwindcss()],
  },
});