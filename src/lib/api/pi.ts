import { invoke } from "@tauri-apps/api/core";
import type { UsageScript } from "@/types";

export interface PiCurrentState {
  enabledProviderIds: string[];
  defaultProviderId: string | null;
}

export const piApi = {
  async getCurrentState(): Promise<PiCurrentState> {
    return await invoke("get_pi_current_state");
  },

  async updateProviderUsageScript(
    id: string,
    usageScript: UsageScript,
  ): Promise<boolean> {
    return await invoke("update_pi_provider_usage_script", {
      id,
      usageScript,
    });
  },
};
