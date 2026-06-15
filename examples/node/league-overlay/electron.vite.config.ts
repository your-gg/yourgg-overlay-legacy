import { defineConfig } from 'electron-vite';

export default defineConfig({
  main: {
    build: {
      lib: {
        entry: './main/index.ts',
      },
      rollupOptions: {
        external: [
          '@your-gg/yourgg-core',
        ],
      },
    },
  },
});
