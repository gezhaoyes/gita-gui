<script setup lang="ts">
import { ref, watch } from "vue";
import {
  NAlert,
  NButton,
  NDynamicTags,
  NForm,
  NFormItem,
  NInput,
  NInputGroup,
  NInputNumber,
  NModal,
  NSpace,
  NSwitch,
  useMessage,
} from "naive-ui";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, errText, type AppConfig } from "../api";
import { store } from "../store";

const props = defineProps<{ show: boolean; firstRun: boolean }>();
const emit = defineEmits<{ "update:show": [boolean]; saved: [] }>();
const message = useMessage();

const form = ref<AppConfig | null>(null);
const busy = ref(false);
// 开机启动不在 config.json 里，打开对话框时向系统查询；null 表示查询失败，不显示开关
const autostart = ref<boolean | null>(null);
let autostartLoaded: boolean | null = null;

watch(
  () => props.show,
  (v) => {
    if (!v || !store.config) return;
    const { default_manifest_dir, ...cfg } = store.config;
    form.value = { ...cfg, skip_dirs: [...cfg.skip_dirs], manifest_dir: cfg.manifest_dir ?? default_manifest_dir };
    autostart.value = autostartLoaded = null;
    api.getAutostart().then(
      (on) => (autostart.value = autostartLoaded = on),
      () => {},
    );
  },
);

async function browse() {
  const picked = await openDialog({ directory: true, title: "选择清单目录", defaultPath: form.value?.manifest_dir ?? undefined });
  if (typeof picked === "string" && form.value) form.value.manifest_dir = picked;
}

async function save() {
  if (!form.value) return;
  const f = form.value;
  busy.value = true;
  try {
    await api.saveConfig({
      ...f,
      manifest_dir: f.manifest_dir?.trim() || null,
      wsl_distro: f.wsl_distro.trim() || "Ubuntu",
      editor: f.editor.trim() || "code",
      gita_windows: f.gita_windows.trim() || "gita",
      discover_depth: f.discover_depth ?? 6,
    });
    if (autostart.value !== null && autostart.value !== autostartLoaded) {
      await api.setAutostart(autostart.value);
    }
    message.success("设置已保存");
    emit("update:show", false);
    emit("saved");
  } catch (e) {
    message.error(errText(e));
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <NModal
    :show="show"
    @update:show="(v: boolean) => emit('update:show', v)"
    preset="card"
    :title="firstRun ? '首次使用：确认清单目录' : '设置'"
    style="width: 640px; max-width: 92vw"
  >
    <NAlert v-if="firstRun" type="info" :show-icon="false" style="margin-bottom: 16px">
      工具会根据两边的 gita 登记，自动维护 manifest-windows.json 和 manifest-wsl.json 两份清单，存在下面的目录里。
      放在 OneDrive 里就会自动同步到云端，换电脑后可以从清单一键 clone 回来。
    </NAlert>
    <NForm v-if="form" label-placement="left" label-width="120" class="settings">
      <NFormItem label="清单目录" :show-feedback="true" feedback="存放 manifest-windows.json 和 manifest-wsl.json。放在 OneDrive 里会自动同步，换电脑后从这里恢复">
        <NInputGroup>
          <NInput v-model:value="form.manifest_dir" placeholder="例如 D:\OneDrive\gita-gui" />
          <NButton @click="browse">浏览</NButton>
        </NInputGroup>
      </NFormItem>
      <template v-if="!firstRun">
        <NFormItem v-if="autostart !== null" label="开机自动启动" :show-feedback="true" feedback="登录 Windows 后自动打开本工具">
          <NSwitch v-model:value="autostart" />
        </NFormItem>
        <NFormItem label="WSL 发行版" :show-feedback="true" feedback="读取哪个 WSL 系统里的仓库，名字和 wsl -l 列出的一致">
          <NInput v-model:value="form.wsl_distro" />
        </NFormItem>
        <NFormItem label="编辑器命令" :show-feedback="true" feedback="右键“在 VS Code 中打开”时调用的命令，例如 code、cursor">
          <NInput v-model:value="form.editor" />
        </NFormItem>
        <NFormItem label="Windows gita" :show-feedback="true" feedback="Windows 侧调用 gita 的命令；不在 PATH 里时填 gita.exe 的完整路径">
          <NInput v-model:value="form.gita_windows" />
        </NFormItem>
        <NFormItem label="查找时跳过" :show-feedback="true" feedback="添加仓库时往下查找，遇到这些名字的文件夹不进去（依赖、编译输出等）">
          <NDynamicTags v-model:value="form.skip_dirs" />
        </NFormItem>
        <NFormItem label="查找深度" :show-feedback="true" feedback="从选中的文件夹往下最多找几层；找到仓库后不再往它里面找">
          <NInputNumber v-model:value="form.discover_depth" :min="1" :max="20" />
        </NFormItem>
      </template>
    </NForm>
    <template #footer>
      <NSpace justify="end">
        <NButton @click="emit('update:show', false)">{{ firstRun ? "以后再说" : "取消" }}</NButton>
        <NButton type="primary" :loading="busy" @click="save">保存</NButton>
      </NSpace>
    </template>
  </NModal>
</template>

<style scoped>
.settings :deep(.n-form-item-feedback) {
  color: var(--n-feedback-text-color, inherit);
  opacity: 0.55;
  font-size: 12px;
}
</style>
