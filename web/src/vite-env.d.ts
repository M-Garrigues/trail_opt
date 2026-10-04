/// <reference types="vite/client" />
/// <reference types="svelte" />
interface ImportMetaEnv { readonly VITE_TURNSTILE_SITEKEY?: string }
declare module '*?worker&url' { const url: string; export default url; }
