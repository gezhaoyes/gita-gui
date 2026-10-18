<script setup lang="ts">
import { computed, h, ref, watch } from "vue";
import { NButton, NEllipsis, NModal, NSpace, NSpin, NTag, useDialog, useMessage } from "naive-ui";
import { api, ENV_LABEL, errText, rowKey, type Env, type PushResult, type RepoRow } from "../api";

const props = defineProps<{ show: boolean; rows: RepoRow[] }>();
const emit = defineEmits<{ "update:show": [boolean]; done: [Env[]] }>();
const message = useMessage();
const dialog = useDialog();

type State = "waiting" | "running" | "ok" | "fail";
interface Item {
  row: RepoRow;
  state: State;
  result: PushResult | null;
}

const items = ref<Item[]>([]);
let runToken = 0;

const running = computed(() => items.value.some((i) => i.state === "running" || i.state === "waiting"));
const summary = computed(() => {
  const ok = items.value.filter((i) => i.state === "ok").length;
  const fail = items.value.filter((i) => i.state === "fail").length;
  return `成功 ${ok}，失败 ${fail}，共 ${items.value.length}`;
});

watch(
  () => props.show,
  (v) => {
    if (!v) return;
    items.value = props.rows.map((row) => {
      // 没有上游的不自动推，等用户确认后再设置上游
      const noUpstream = !!row.status?.branch && !row.status.upstream;
      return {
        row,
        state: noUpstream ? "fail" : "waiting",
        result: noUpstream ? { ok: false, kind: "no_upstream", message: "当前分支没有设置上游" } : null,
      };
    });
    runQueue();
  },
);

// 一个接一个推，避免同时弹出多个登录框
async function runQueue() {
  const token = ++runToken;
  for (const item of items.value) {
    if (token !== runToken) return;
    if (item.state !== "waiting") continue;
    await pushOne(item, false);
  }
}

async function pushOne(item: Item, setUpstream: boolean) {
  item.state = "running";
  try {
    const r = setUpstream
      ? await api.pushSetUpstream(item.row.env, item.row.path)
      : await api.push(item.row.env, item.row.path);
    item.result = r;
    item.state = r.ok ? "ok" : "fail";
  } catch (e) {
    item.result = { ok: false, kind: "other", message: errText(e) };
    item.state = "fail";
  }
}

async function retry(item: Item) {
  await pushOne(item, false);
}

function setUpstream(item: Item) {
  const branch = item.row.status?.branch ?? "";
  dialog.info({
    title: "设置上游并推送",
    content: `将 ${item.row.name} 的分支 ${branch} 推送到远程同名分支，并设为上游（git push -u）。`,
    positiveText: "推送",
    negativeText: "取消",
    onPositiveClick: () => {
      pushOne(item, true);
    },
  });
}

async function setupAuth() {
  try {
    const helper = await api.setupWslAuth();
    message.success(`已配置 WSL 使用 Windows 的凭据管理器：${helper}`);
  } catch (e) {
    dialog.error({ title: "一键配置未完成", content: () => h("pre", { style: { whiteSpace: "pre-wrap", fontFamily: "Consolas, monospace", fontSize: "12px", userSelect: "text" } }, errText(e)) });
  }
}

async function openEditor(item: Item) {
  try {
    await api.openInEditor(item.row.env, item.row.path);
  } catch (e) {
    message.error(errText(e));
  }
}

const INTEROP_FIX =
  "sudo sh -c 'echo :WSLInterop:M::MZ::/init:PF > /proc/sys/fs/binfmt_misc/register'";

const KIND_TEXT = {
  interop: "WSL 无法调用凭据管理器",
  auth: "认证失败",
  no_upstream: "没有上游",
  rejected: "被远程拒绝",
  network: "网络问题",
  other: "失败",
} as const;

function hint(item: Item): string {
  switch (item.result?.kind) {
    case "interop":
      return "WSL 运行不了 Windows 程序（常见于开启 systemd 后 WSLInterop 注册丢失），所以调不起凭据管理器。请在 WSL 终端执行下面这行（需要 sudo 密码），然后重试。它在重启 WSL 后会失效，永久修复见项目 README：";
    case "auth":
      return item.row.env === "wsl"
        ? "WSL 侧未配置 GitHub 认证。可以一键配置为借用 Windows 的凭据管理器，然后重试。"
        : "Windows 侧认证失败。重试时会弹出 Git Credential Manager 的登录窗口；如果没有弹出，请在终端里对这个仓库执行一次 git push 完成登录。";
    case "rejected":
      return "远程有本地没有的提交，请在编辑器里先拉取并合并，再推送。";
    case "network":
      return "连不上远程仓库，检查网络或代理后重试。";
    case "no_upstream":
      return "当前分支还没有对应的远程分支。";
    default:
      return "";
  }
}

function close() {
  emit("update:show", false);
  emit("done", [...new Set(items.value.map((i) => i.row.env))]);
}
</script>

<template>
  <NModal
    :show="show"
    @update:show="(v: boolean) => !v && close()"
    preset="card"
    title="推送"
    style="width: 820px; max-width: 94vw"
  >
    <div class="list">
      <div v-for="item in items" :key="rowKey(item.row)" class="item">
        <div class="head">
          <NSpin v-if="item.state === 'running'" :size="14" />
          <NTag v-else-if="item.state === 'ok'" size="small" type="success" :bordered="false">成功</NTag>
          <NTag v-else-if="item.state === 'fail'" size="small" type="error" :bordered="false">
            {{ KIND_TEXT[item.result?.kind ?? "other"] }}
          </NTag>
          <NTag v-else size="small" :bordered="false">等待</NTag>
          <NTag size="small" :bordered="false" :type="item.row.env === 'wsl' ? 'info' : 'default'">
            {{ ENV_LABEL[item.row.env] }}
          </NTag>
          <b>{{ item.row.name }}</b>
          <span class="path">{{ item.row.path }}</span>
          <span class="grow" />
          <template v-if="item.state === 'fail'">
            <NButton v-if="item.result?.kind === 'no_upstream'" size="tiny" type="primary" @click="setUpstream(item)">
              设置上游并推送
            </NButton>
            <NButton
              v-if="item.result?.kind === 'auth' && item.row.env === 'wsl'"
              size="tiny"
              type="primary"
              @click="setupAuth"
            >
              一键配置
            </NButton>
            <NButton v-if="item.result?.kind === 'rejected'" size="tiny" @click="openEditor(item)">
              在 VS Code 中打开
            </NButton>
            <NButton v-if="item.result?.kind !== 'no_upstream'" size="tiny" @click="retry(item)">重试</NButton>
          </template>
        </div>
        <div v-if="item.state === 'fail'" class="detail">
          <div v-if="hint(item)" class="hint">{{ hint(item) }}</div>
          <pre v-if="item.result?.kind === 'interop'" class="fix">{{ INTEROP_FIX }}</pre>
          <NEllipsis v-if="item.result?.message" :line-clamp="3" class="msg">{{ item.result.message }}</NEllipsis>
        </div>
      </div>
    </div>
    <template #footer>
      <NSpace justify="space-between" align="center">
        <span class="summary">{{ running ? "推送中…" : summary }}</span>
        <NButton @click="close">关闭</NButton>
      </NSpace>
    </template>
  </NModal>
</template>

<style scoped>
.list {
  max-height: 60vh;
  overflow: auto;
}
.item {
  padding: 6px 0;
  border-bottom: 1px solid rgba(128, 128, 128, 0.15);
}
.head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.path {
  opacity: 0.6;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}
.grow {
  flex: 1;
}
.detail {
  margin: 4px 0 0 4px;
  font-size: 12px;
}
.hint {
  color: #f0a020;
  margin-bottom: 2px;
}
.msg {
  font-family: Consolas, monospace;
  opacity: 0.75;
  white-space: pre-wrap;
}
.fix {
  margin: 4px 0;
  padding: 6px 8px;
  border-radius: 4px;
  background: rgba(128, 128, 128, 0.12);
  font-family: Consolas, monospace;
  font-size: 12px;
  white-space: pre-wrap;
  user-select: text;
}
.summary {
  font-size: 13px;
  opacity: 0.8;
}
</style>
