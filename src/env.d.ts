/// <reference types="vite/client" />

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<{}, {}, any>;
  export default component;
}

declare module "vue-virtual-scroller" {
  import { Component } from "vue";
  export const RecycleScroller: Component;
  export const DynamicScroller: Component;
  export const DynamicScrollerItem: Component;
}
