import { getVersion } from "@tauri-apps/api/app";

export type UpdateChannel = "stable" | "beta";

export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string;
  notes?: string;
  pubDate?: string;
}

export interface CheckOptions {
  timeout?: number;
  channel?: UpdateChannel;
}

export async function getCurrentVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

export async function checkForUpdate(
  opts: CheckOptions = {},
): Promise<
  | { status: "up-to-date" }
  | { status: "available"; info: UpdateInfo }
  | { status: "unsupported" }
> {
  // 动态引入，避免在未安装插件时导致打包期问题
  const { check } = await import("@tauri-apps/plugin-updater");

  const currentVersion = await getCurrentVersion();

  let update: { version?: string; notes?: string; date?: string } | null;
  try {
    update = await check({ timeout: opts.timeout ?? 30000 } as any);
  } catch (error) {
    // 便携版与开发构建没有内置更新源，这是预期状态而非故障：
    // 交给调用方提示用户前往 Releases 页面，而不是弹一个假的错误。
    if (isUpdaterUnconfigured(error)) {
      return { status: "unsupported" };
    }
    throw error;
  }

  if (!update) {
    return { status: "up-to-date" };
  }

  const info: UpdateInfo = {
    currentVersion,
    availableVersion: update.version ?? "",
    notes: update.notes,
    pubDate: update.date,
  };

  return { status: "available", info };
}

/**
 * 判定「更新源未配置」类错误。
 *
 * 对应 tauri-plugin-updater 的 `Error::EmptyEndpoints`
 * （原文 "Updater does not have any endpoints set."）。
 */
function isUpdaterUnconfigured(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return message.toLowerCase().includes("does not have any endpoints");
}
