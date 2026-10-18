<script setup lang="ts">
import { computed } from "vue";
import {
  NConfigProvider,
  NDialogProvider,
  NGlobalStyle,
  NMessageProvider,
  darkTheme,
  dateZhCN,
  useOsTheme,
  zhCN,
} from "naive-ui";
import MainView from "./views/MainView.vue";

const os = useOsTheme();
const theme = computed(() => (os.value === "dark" ? darkTheme : null));
</script>

<template>
  <NConfigProvider :theme="theme" :locale="zhCN" :date-locale="dateZhCN">
    <NGlobalStyle />
    <NMessageProvider>
      <NDialogProvider>
        <MainView />
      </NDialogProvider>
    </NMessageProvider>
  </NConfigProvider>
</template>
