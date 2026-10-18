<script setup lang="ts">
import { computed, h, nextTick, onMounted, onUnmounted, ref } from "vue";
import {
  NButton,
  NDataTable,
  NDropdown,
  NEllipsis,
  NInput,
  NSelect,
  NSpace,
  NSpin,
  NSwitch,
  NTag,
  NTooltip,
  useDialog,
  useMessage,
  type DataTableColumns,
  type DropdownOption,
} from "naive-ui";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { api, ENV_LABEL, errText, rowKey, type DiscoverResult, type Env, type RepoRow } from "../api";
import { allGroups, allRows, loadAll, loadConfig, loadEnv, store } from "../store";
import AddDialog from "./AddDialog.vue";
import PushDialog from "./PushDialog.vue";
import ManifestDialog from "./ManifestDialog.vue";
import SettingsDialog from "./SettingsDialog.vue";

const message = useMessage();
const dialog = useDialog();

// ---------- 筛选 ----------
const envFilter = ref<Env | "all">("all");
const groupFilter = ref<string | null>(null);
const onlyUnsafe = ref(false);
const keyword = ref("");
const NO_GROUP = "\u0000none";

const envOptions = [
  { label: "全部环境", value: "all" },
  { label: "Windows", value: "windows" },
  { label: "WSL", value: "wsl" },
];
const groupOptions = computed(() => [
  { label: "（无分组）", value: NO_GROUP },
  ...allGroups.value.map((g) => ({ label: g, value: g })),
]);

const rows = computed(() => {
  const kw = keyword.value.trim().toLowerCase();
  return allRows.value.filter((r) => {
    if (envFilter.value !== "all" && r.env !== envFilter.value) return false;
    if (groupFilter.value === NO_GROUP && r.groups.length) return false;
    if (groupFilter.value && groupFilter.value !== NO_GROUP && !r.groups.includes(groupFilter.value)) return false;
    if (onlyUnsafe.value && r.color === "green") return false;
    if (kw && !r.name.toLowerCase().includes(kw) && !r.path.toLowerCase().includes(kw)) return false;
    return true;
  });
});

const checked = ref<string[]>([]);
const checkedRows = computed(() => {
  const set = new Set(checked.value);
  return allRows.value.filter((r) => set.has(rowKey(r)));
});

// ---------- 表格 ----------
const COLOR_RANK = { gray: 3, red: 2, yellow: 1, green: 0 } as const;
const COLOR_CSS = { green: "#18a058", yellow: "#f0a020", red: "#d03050", gray: "#9a9a9a" } as const;
const COLOR_TEXT = { green: "已备份", yellow: "有改动", red: "未备份", gray: "路径不存在" } as const;

function needsPush(r: RepoRow): boolean {
  const s = r.status;
  if (!s || !s.remotes.length || !s.branch) return false;
  return s.ahead > 0 || !s.upstream || s.upstream_gone || !!r.push_failed;
}

const columns = computed<DataTableColumns<RepoRow>>(() => [
  { type: "selection" },
  {
    title: "状态",
    key: "color",
    width: 120,
    sorter: (a, b) => COLOR_RANK[a.color] - COLOR_RANK[b.color],
    render: (r) =>
      h(NTooltip, null, {
        trigger: () =>
          h("span", { class: "state" }, [
            h("span", { class: "dot", style: { background: COLOR_CSS[r.color] } }),
            h("span", { style: { color: r.color === "green" ? undefined : COLOR_CSS[r.color] } }, COLOR_TEXT[r.color]),
          ]),
        default: () =>
          h("div", [h("b", COLOR_TEXT[r.color]), ...r.reasons.map((x) => h("div", x))]),
      }),
  },
  {
    title: "环境",
    key: "env",
    width: 90,
    sorter: (a, b) => a.env.localeCompare(b.env),
    render: (r) => h(NTag, { size: "small", bordered: false, type: r.env === "wsl" ? "info" : "default" }, () => ENV_LABEL[r.env]),
  },
  {
    title: "名字",
    key: "name",
    minWidth: 140,
    sorter: (a, b) => a.name.localeCompare(b.name),
    render: (r) =>
      h(NTooltip, { placement: "top-start" }, {
        trigger: () => h("span", { style: r.exists ? undefined : { textDecoration: "line-through", opacity: 0.6 } }, r.name),
        default: () => r.path,
      }),
  },
  {
    title: "分组",
    key: "groups",
    width: 140,
    render: (r) =>
      h(NSpace, { size: 4 }, () => r.groups.map((g) => h(NTag, { size: "small", round: true }, () => g))),
  },
  {
    title: "分支",
    key: "branch",
    width: 130,
    render: (r) => {
      const s = r.status;
      if (!s) return "";
      if (!s.branch) return h(NTag, { size: "small", type: "error", bordered: false }, () => "分离 HEAD");
      return h(NEllipsis, null, () => s.branch);
    },
  },
  {
    title: "未推送",
    key: "ahead",
    width: 90,
    sorter: (a, b) => (a.status?.ahead ?? 0) - (b.status?.ahead ?? 0),
    render: (r) => {
      const s = r.status;
      if (!s) return "";
      if (s.branch && s.remotes.length && !s.upstream)
        return h(NTag, { size: "small", type: "error", bordered: false }, () => "无上游");
      if (s.upstream_gone)
        return h(NTag, { size: "small", type: "error", bordered: false }, () => "上游已不存在");
      return s.ahead ? h("b", { style: { color: COLOR_CSS.red } }, String(s.ahead)) : h("span", { class: "muted" }, "0");
    },
  },
  {
    title: "未提交",
    key: "dirty",
    width: 80,
    render: (r) => {
      if (!r.status) return "";
      return r.status.dirty
        ? h(NTag, { size: "small", type: "warning", bordered: false }, () => "有改动")
        : h("span", { class: "muted" }, "无");
    },
  },
  {
    title: "远程",
    key: "remote",
    minWidth: 200,
    render: (r) => {
      if (r.error) return h(NEllipsis, { style: { color: COLOR_CSS.red } }, () => r.error);
      if (!r.status) return "";
      if (!r.remote_url) return h("span", { style: { color: COLOR_CSS.red } }, "没有远程仓库");
      const url = r.remote_url;
      // 只推到拉取地址的普通情况
      if (r.push_urls.length === 1 && r.push_urls[0] === url) return h(NEllipsis, null, () => url);
      const extra = r.push_urls.filter((u) => u !== url).length;
      const skipsFetchUrl = !r.push_urls.includes(url);
      return h(NTooltip, { placement: "top-end" }, {
        trigger: () =>
          h("span", { class: "remote-cell" }, [
            h(NEllipsis, { style: { minWidth: 0 } }, () => url),
            extra ? h(NTag, { size: "small", round: true, bordered: false, type: "info" }, () => `+${extra}`) : null,
          ]),
        default: () =>
          h("div", [
            h("div", `拉取：${url}`),
            h("div", { style: { marginTop: "4px" } }, `推送到 ${r.push_urls.length} 个地址：`),
            ...r.push_urls.map((u) => h("div", `· ${u}`)),
            skipsFetchUrl
              ? h("div", { style: { color: COLOR_CSS.yellow, marginTop: "4px" } }, "注意：不会推送到拉取地址")
              : null,
          ]),
      });
    },
  },
  {
    title: "",
    key: "actions",
    width: 76,
    render: (r) =>
      needsPush(r)
        ? h(NButton, { size: "tiny", type: "primary", secondary: true, onClick: () => startPush([r]) }, () => "推送")
        : "",
  },
]);

// ---------- 右键菜单 ----------
const menu = ref({ show: false, x: 0, y: 0, row: null as RepoRow | null });
const menuOptions = computed<DropdownOption[]>(() => {
  const r = menu.value.row;
  return [
    { label: "在资源管理器中打开", key: "explorer", disabled: !r?.exists },
    { label: "在 VS Code 中打开", key: "editor", disabled: !r?.exists },
    { type: "divider", key: "d1" },
    { label: "推送", key: "push", disabled: !r || !needsPush(r) },
    { label: "重新定位…", key: "relocate" },
    { label: "移除登记", key: "remove" },
  ];
});

function rowProps(row: RepoRow) {
  return {
    onContextmenu: (e: MouseEvent) => {
      e.preventDefault();
      menu.value.show = false;
      nextTick(() => {
        menu.value = { show: true, x: e.clientX, y: e.clientY, row };
      });
    },
  };
}

async function onMenuSelect(key: string) {
  menu.value.show = false;
  const r = menu.value.row;
  if (!r) return;
  try {
    if (key === "explorer") await api.openInExplorer(r.env, r.path);
    else if (key === "editor") await api.openInEditor(r.env, r.path);
    else if (key === "push") startPush([r]);
    else if (key === "relocate") await relocate(r);
    else if (key === "remove") confirmRemove(r);
  } catch (e) {
    message.error(errText(e));
  }
}

function wslRoot(): string {
  return `\\\\wsl.localhost\\${store.config?.wsl_distro ?? "Ubuntu"}`;
}

async function relocate(r: RepoRow) {
  const picked = await openDialog({
    directory: true,
    title: `为 ${r.name} 选择新位置`,
    defaultPath: r.env === "wsl" ? wslRoot() : undefined,
  });
  if (typeof picked !== "string") return;
  await api.relocateRepo(r.env, r.name, picked);
  message.success(`已重新定位 ${r.name}`);
  await loadEnv(r.env);
}

function confirmRemove(r: RepoRow) {
  dialog.warning({
    title: "移除登记",
    content: `从 gita 中移除 ${r.name}？只取消登记，不会删除文件。`,
    positiveText: "移除",
    negativeText: "取消",
    onPositiveClick: async () => {
      try {
        await api.removeRepo(r.env, r.name);
        message.success(`已移除 ${r.name}`);
        await loadEnv(r.env);
      } catch (e) {
        message.error(errText(e));
      }
    },
  });
}

// ---------- 添加 ----------
const addShow = ref(false);
const addResult = ref<DiscoverResult | null>(null);

async function pickFolder() {
  const picked = await openDialog({ directory: true, multiple: true, title: "选择仓库或包含仓库的文件夹" });
  if (!picked) return;
  await handleAddPaths(Array.isArray(picked) ? picked : [picked]);
}

async function handleAddPaths(paths: string[]) {
  const msg = message.loading("正在查找仓库…", { duration: 0 });
  try {
    const results: DiscoverResult[] = [];
    for (const p of paths) results.push(await api.discover(p));
    const env = results[0].env;
    if (results.some((r) => r.env !== env)) {
      message.error("一次只能添加同一边（Windows 或 WSL）的仓库");
      return;
    }
    // 选中的文件夹本身就是仓库：直接加入
    if (results.length === 1 && results[0].root_is_repo) {
      const only = results[0].found[0];
      if (only.registered) {
        message.info("这个仓库已经登记过了");
        return;
      }
      await api.addRepos(env, [only.path], null);
      message.success(`已加入 ${only.path}`);
      await loadEnv(env);
      return;
    }
    const found = results.flatMap((r) => r.found);
    if (!found.length) {
      message.warning("没有找到 git 仓库");
      return;
    }
    addResult.value = { env, root: results.map((r) => r.root).join("、"), root_is_repo: false, found };
    addShow.value = true;
  } catch (e) {
    message.error(errText(e));
  } finally {
    msg.destroy();
  }
}

// ---------- 推送 ----------
const pushShow = ref(false);
const pushRows = ref<RepoRow[]>([]);

function startPush(list: RepoRow[]) {
  const targets = list.filter(needsPush);
  if (!targets.length) {
    message.info("选中的仓库没有需要推送的内容");
    return;
  }
  pushRows.value = targets;
  pushShow.value = true;
}

const pushAllCount = computed(() => allRows.value.filter(needsPush).length);

// ---------- 清单 ----------
const manifestShow = ref(false);
const manifestEnv = ref<Env>("windows");

function openManifest(env: Env) {
  manifestEnv.value = env;
  manifestShow.value = true;
}

function confirmOverwrite(env: Env) {
  const m = store.envs[env].manifest;
  const removed = m?.state === "conflict" ? m.only_in_file : [];
  dialog.warning({
    title: `用本机覆盖 ${ENV_LABEL[env]} 清单`,
    content: () =>
      h("div", [
        h("p", "清单文件被别的电脑或手工修改过。覆盖后，清单内容会和本机的 gita 登记一致。"),
        removed.length
          ? h("div", [
              h("p", `下面 ${removed.length} 个仓库只在清单里有，覆盖后会从清单中去掉：`),
              h("ul", { style: { maxHeight: "200px", overflow: "auto" } }, removed.map((p) => h("li", p))),
            ])
          : null,
      ]),
    positiveText: "覆盖",
    negativeText: "取消",
    onPositiveClick: async () => {
      try {
        store.envs[env].manifest = await api.overwriteManifest(env);
        message.success("清单已更新");
      } catch (e) {
        message.error(errText(e));
      }
    },
  });
}

// ---------- 设置 ----------
const settingsShow = ref(false);
const firstRun = ref(false);

async function onSettingsSaved() {
  await loadConfig();
  if (firstRun.value) {
    firstRun.value = false;
  }
  await loadAll();
}

// ---------- 启动 ----------
let unlistenDrop: UnlistenFn | null = null;
let unlistenFocus: UnlistenFn | null = null;
const dragging = ref(false);

// 切回窗口时自动刷新，捕捉在编辑器、终端里做的提交和推送；两次自动刷新至少间隔 15 秒
const FOCUS_REFRESH_GAP = 15_000;
let lastAutoRefresh = Date.now();

function refreshOnFocus() {
  if (Date.now() - lastAutoRefresh < FOCUS_REFRESH_GAP) return;
  if (pushShow.value || manifestShow.value || addShow.value) return;
  lastAutoRefresh = Date.now();
  loadAll();
}

onMounted(async () => {
  try {
    await loadConfig();
  } catch (e) {
    message.error(errText(e));
  }
  if (store.config && !store.config.manifest_dir) {
    firstRun.value = true;
    settingsShow.value = true;
  }
  loadAll();
  unlistenDrop = await getCurrentWebview().onDragDropEvent((event) => {
    const p = event.payload;
    if (p.type === "enter" || p.type === "over") dragging.value = true;
    else dragging.value = false;
    if (p.type === "drop" && p.paths.length) handleAddPaths(p.paths);
  });
  unlistenFocus = await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
    if (focused) refreshOnFocus();
  });
});

onUnmounted(() => {
  unlistenDrop?.();
  unlistenFocus?.();
});

function envSummary(env: Env) {
  const s = store.envs[env];
  if (s.loading && !s.loaded) return "加载中…";
  const red = s.rows.filter((r) => r.color === "red").length;
  return `${s.rows.length} 个仓库${red ? `，${red} 个未备份` : ""}`;
}
</script>

<template>
  <div class="layout">
    <header class="toolbar">
      <NSpace align="center" :size="8">
        <NButton @click="loadAll" :loading="store.envs.windows.loading || store.envs.wsl.loading">刷新</NButton>
        <NButton @click="pickFolder">添加</NButton>
        <NButton type="primary" :disabled="!checkedRows.length" @click="startPush(checkedRows)">
          推送选中{{ checkedRows.length ? ` (${checkedRows.length})` : "" }}
        </NButton>
        <NButton type="primary" secondary :disabled="!pushAllCount" @click="startPush(allRows)">
          推送全部未推送{{ pushAllCount ? ` (${pushAllCount})` : "" }}
        </NButton>
        <NButton @click="openManifest('windows')">从清单恢复</NButton>
        <NButton quaternary @click="settingsShow = true">设置</NButton>
      </NSpace>
      <NSpace align="center" :size="8">
        <NInput v-model:value="keyword" placeholder="搜索名字或路径" clearable size="small" style="width: 180px" />
        <NSelect v-model:value="envFilter" :options="envOptions" size="small" style="width: 110px" />
        <NSelect
          v-model:value="groupFilter"
          :options="groupOptions"
          placeholder="全部分组"
          clearable
          size="small"
          style="width: 130px"
        />
        <NSpace align="center" :size="4" :wrap="false">
          <NSwitch v-model:value="onlyUnsafe" size="small" />
          <span class="label">只看没备份的</span>
        </NSpace>
      </NSpace>
    </header>

    <main class="table">
      <NDataTable
        :columns="columns"
        :data="rows"
        :row-key="rowKey"
        :row-props="rowProps"
        v-model:checked-row-keys="checked"
        size="small"
        flex-height
        style="height: 100%"
        :bordered="false"
      >
        <template #empty>
          <div class="empty">
            <NSpin v-if="store.envs.windows.loading && !store.envs.windows.loaded" size="small" />
            <span v-else>还没有登记的仓库。点"添加"或者把文件夹拖进窗口。</span>
          </div>
        </template>
      </NDataTable>
      <div v-if="dragging" class="drop-mask">松开以添加仓库</div>
    </main>

    <footer class="statusbar">
      <div v-for="env in (['windows', 'wsl'] as Env[])" :key="env" class="env-status">
        <b>{{ ENV_LABEL[env] }}</b>
        <NSpin v-if="store.envs[env].loading" :size="12" />
        <span v-if="store.envs[env].error" class="err" :title="store.envs[env].error!">
          读取失败：{{ store.envs[env].error }}
        </span>
        <span v-else>{{ envSummary(env) }}</span>
        <template v-if="store.envs[env].manifest">
          <span class="sep">·</span>
          <template v-if="store.envs[env].manifest!.state === 'conflict'">
            <span class="warn">清单被别处修改过，未覆盖</span>
            <NButton size="tiny" text type="primary" @click="openManifest(env)">从清单恢复</NButton>
            <NButton size="tiny" text type="warning" @click="confirmOverwrite(env)">用本机覆盖</NButton>
          </template>
          <span v-else-if="store.envs[env].manifest!.state === 'not_configured'" class="warn">
            未设置清单目录
            <NButton size="tiny" text type="primary" @click="settingsShow = true">设置</NButton>
          </span>
          <span v-else-if="store.envs[env].manifest!.state === 'error'" class="err">
            清单写入失败：{{ (store.envs[env].manifest as any).message }}
          </span>
          <span v-else>清单已同步</span>
        </template>
      </div>
    </footer>

    <NDropdown
      trigger="manual"
      placement="bottom-start"
      :show="menu.show"
      :x="menu.x"
      :y="menu.y"
      :options="menuOptions"
      @select="onMenuSelect"
      @clickoutside="menu.show = false"
    />

    <AddDialog v-model:show="addShow" :result="addResult" @added="(env: Env) => loadEnv(env)" />
    <PushDialog v-model:show="pushShow" :rows="pushRows" @done="(envs: Env[]) => envs.forEach(loadEnv)" />
    <ManifestDialog v-model:show="manifestShow" v-model:env="manifestEnv" @restored="(env: Env) => loadEnv(env)" />
    <SettingsDialog v-model:show="settingsShow" :first-run="firstRun" @saved="onSettingsSaved" />
  </div>
</template>

<style scoped>
.layout {
  display: flex;
  flex-direction: column;
  height: 100vh;
}
.toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--n-border-color, rgba(128, 128, 128, 0.2));
}
.label {
  font-size: 13px;
  white-space: nowrap;
}
.table {
  flex: 1;
  min-height: 0;
  position: relative;
  padding: 0 4px;
}
.empty {
  padding: 40px 0;
  opacity: 0.7;
}
.drop-mask {
  position: absolute;
  inset: 8px;
  border: 2px dashed #2080f0;
  border-radius: 8px;
  background: rgba(32, 128, 240, 0.08);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 18px;
  pointer-events: none;
}
.statusbar {
  display: flex;
  gap: 24px;
  padding: 6px 12px;
  font-size: 12px;
  border-top: 1px solid rgba(128, 128, 128, 0.2);
  flex-wrap: wrap;
}
.env-status {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.sep {
  opacity: 0.5;
}
.warn {
  color: #f0a020;
}
.err {
  color: #d03050;
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
:deep(.state) {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
}
:deep(.remote-cell) {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
:deep(.muted) {
  opacity: 0.45;
}
:deep(.dot) {
  display: inline-block;
  width: 10px;
  height: 10px;
  border-radius: 50%;
}
</style>
