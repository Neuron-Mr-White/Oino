import { mergeConfig } from 'vite';
import base from './vite.config.capture';

export default mergeConfig(base, { server: { watch: { ignored: ['**/target/**', '**/src-tauri/**', '**/.git/**'] } } });
