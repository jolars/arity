import {
  Fragment,
  defineComponent,
  h,
  nextTick,
  onMounted,
  onUnmounted,
  watch,
} from "vue";
import { useData, useRoute } from "vitepress";
import DefaultTheme from "vitepress/theme";
import { mountCharts } from "./bench-charts.mjs";
import "./style.css";
import "./bench-charts.css";

const Charts = defineComponent({
  setup() {
    const route = useRoute();
    const { isDark } = useData();
    let dispose;
    let stop;
    onMounted(() => {
      stop = watch(
        [() => route.path, isDark],
        async () => {
          dispose?.();
          await nextTick();
          dispose = mountCharts();
        },
        { immediate: true, flush: "post" },
      );
    });
    onUnmounted(() => {
      stop?.();
      dispose?.();
    });
    return () => null;
  },
});

export default {
  extends: DefaultTheme,
  Layout: () => h(Fragment, [h(DefaultTheme.Layout), h(Charts)]),
};
