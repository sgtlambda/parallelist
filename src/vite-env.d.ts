/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** The branch `run.sh` built from, unless that was main. */
  readonly VITE_PARALLELIST_BRANCH?: string
}

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<{}, {}, any>;
  export default component;
}
