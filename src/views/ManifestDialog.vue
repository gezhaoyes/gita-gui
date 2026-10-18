<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  NButton,
  NCheckbox,
  NEllipsis,
  NInput,
  NModal,
  NRadioButton,
  NRadioGroup,
  NSpace,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import { api, errText, type CloneResult, type Env, type ManifestEntry } from "../api";
import { store } from "../store";

const props = defineProps<{ show: boolean; env: Env }>();
const emit = defineEmits<{ "update:show": [boolean]; "update:env": [Env]; restored: [Env] }>();
const message = useMessage();

interface Item {
  entry: ManifestEntry;
  target: string;
  checked: boolean;
  registered: boolean;
  running: boolean;
  result: CloneResult | null;
}

const loading = ref(false);
const loadError = ref<string | null>(null);
const items = ref<Item[]>([]);
const busy = ref(false);

function norm(env: Env, p: string) {
  return env === "windows" ? p.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase() : p.replace(/\/+$/, "");
}

async function load() {
  const env = props.env;
  loading.value = true;
  loadError.value = null;
  items.value = [];
  try {
    const m = await api.readManifest(env);
    if (!m) {
      loadError.value = "清单目录里还没有这一边的清单文件";
      return;
    }
    const registered = new Set(store.envs[env].rows.map((r) => norm(env, r.path)));
    items.value = m.repos.map((entry) => {
      const reg = registered.has(norm(env, entry.path));
      return {
        entry,
        target: entry.path,
        registered: reg,
        checked: !reg && !!entry.remote,
        running: false,
        result: null,
      };
    });
  } catch (e) {
    loadError.value = errText(e);
  } finally {
    loading.value = false;
  }
}

watch(
  () => [props.show, props.env] as const,
  ([show]) => {
    if (show) load();
  },
);

const selected = computed(() => items.value.filter((i) => i.checked && i.entry.remote));

async function restore() {
  busy.value = true;
  const env = props.env;
  for (const item of selected.value) {
    item.running = true;
    try {
      item.result = await api.cloneEntry(env, item.entry, item.target.trim());
    } catch (e) {
      item.result = { state: "failed", message: errText(e) };
    }
    item.running = false;
    if (item.result.state !== "failed") item.checked = false;
  }
  busy.value = false;
  const failed = items.value.filter((i) => i.result?.state === "failed").length;
  if (failed) message.warning(`恢复完成，${failed} 个失败`);
  else message.success("恢复完成");
  emit("restored", env);
}

const RESULT_TAG = { cloned: "success", skipped: "default", failed: "error" } as const;
const RESULT_TEXT = { cloned: "已 clone", skipped: "已存在", failed: "失败" } as const;
</script>

<template>
  <NModal
    :show="show"
    @update:show="(v: boolean) => emit('update:show', v)"
    preset="card"
    title="从清单恢复"
    style="width: 1000px; max-width: 96vw"
  >
    <NSpace align="center" justify="space-between" style="margin-bottom: 10px">
      <NRadioGroup :value="env" @update:value="(v: Env) => emit('update:env', v)" size="small" :disabled="busy">
        <NRadioButton value="windows">Windows 清单</NRadioButton>
        <NRadioButton value="wsl">WSL 清单</NRadioButton>
      </NRadioGroup>
      <span class="tip">勾选要恢复的仓库，目标路径可以改。本地已存在的会跳过 clone，只补登记和分组。</span>
    </NSpace>

    <div v-if="loading" class="center"><NSpin size="small" /></div>
    <div v-else-if="loadError" class="center err">{{ loadError }}</div>
    <div v-else class="list">
      <div v-for="(item, i) in items" :key="i" class="item">
        <NCheckbox v-model:checked="item.checked" :disabled="!item.entry.remote || busy" />
        <div class="main">
          <div class="line">
            <b>{{ item.entry.name }}</b>
            <NTag v-for="g in item.entry.groups" :key="g" size="small" round>{{ g }}</NTag>
            <span class="branch" v-if="item.entry.branch">{{ item.entry.branch }}</span>
            <NTag v-if="item.registered" size="small" :bordered="false">本机已登记</NTag>
            <NTag v-if="!item.entry.remote" size="small" type="error" :bordered="false">没有远程，无法恢复</NTag>
            <span class="grow" />
            <NSpin v-if="item.running" :size="14" />
            <NTag v-else-if="item.result" size="small" :type="RESULT_TAG[item.result.state]" :bordered="false">
              {{ RESULT_TEXT[item.result.state] }}
            </NTag>
          </div>
          <div class="line">
            <NInput v-model:value="item.target" size="tiny" :disabled="busy" style="flex: 1" />
            <NEllipsis class="remote">{{ item.entry.remote }}</NEllipsis>
            <NTag
              v-if="item.entry.push_urls?.length"
              size="small"
              round
              :bordered="false"
              type="info"
              :title="`推送到：\n${item.entry.push_urls.join('\n')}`"
            >
              推送 {{ item.entry.push_urls.length }} 处
            </NTag>
          </div>
          <div v-if="item.result?.state === 'failed'" class="err small">{{ item.result.message }}</div>
        </div>
      </div>
      <div v-if="!items.length" class="center">清单是空的</div>
    </div>

    <template #footer>
      <NSpace justify="end">
        <NButton @click="emit('update:show', false)" :disabled="busy">关闭</NButton>
        <NButton type="primary" :disabled="!selected.length" :loading="busy" @click="restore">
          恢复 {{ selected.length }} 个
        </NButton>
      </NSpace>
    </template>
  </NModal>
</template>

<style scoped>
.tip {
  font-size: 12px;
  opacity: 0.7;
}
.center {
  padding: 30px;
  text-align: center;
}
.list {
  max-height: 60vh;
  overflow: auto;
}
.item {
  display: flex;
  gap: 10px;
  padding: 8px 0;
  border-bottom: 1px solid rgba(128, 128, 128, 0.15);
}
.main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.line {
  display: flex;
  align-items: center;
  gap: 8px;
}
.branch {
  font-size: 12px;
  opacity: 0.7;
}
.remote {
  width: 40%;
  font-size: 12px;
  opacity: 0.7;
}
.grow {
  flex: 1;
}
.err {
  color: #d03050;
}
.small {
  font-size: 12px;
  white-space: pre-wrap;
}
</style>
