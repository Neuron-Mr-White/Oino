import { defineConfig } from 'vitest/config';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';
export default defineConfig({
  plugins: [solid(), tailwind(), { name: 'oin38-mock', enforce: 'pre', transform(code, id) {
    if (id.endsWith('/src/components/ComponentsCard.tsx')) return code.replace("import { getVersion } from '@tauri-apps/api/app';", "const getVersion = async () => '1.0.2';");
    if (id.endsWith('/src/lib/host/factory.ts')) return `import '/src/dev/mock/oin38.ts';\nimport '/src/dev/mock/boot.ts';\n${code}`;
  } }],
  server: { port: 1458, strictPort: true, host: '127.0.0.1', watch: { ignored: ['**/target/**', '**/dist/**'] } },
});
