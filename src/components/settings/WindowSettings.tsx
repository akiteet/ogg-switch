import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { SettingsFormState } from "@/hooks/useSettings";
import { AppWindow, Power, EyeOff, MonitorDown } from "lucide-react";
import { ToggleRow } from "@/components/ui/toggle-row";
import { Button } from "@/components/ui/button";
import { AnimatePresence, motion } from "framer-motion";
import { isLinux } from "@/lib/platform";
import { settingsApi } from "@/lib/api/settings";
import { toast } from "sonner";

interface WindowSettingsProps {
  settings: SettingsFormState;
  onChange: (updates: Partial<SettingsFormState>) => void;
}

export function WindowSettings({ settings, onChange }: WindowSettingsProps) {
  const { t } = useTranslation();
  const [creatingShortcut, setCreatingShortcut] = useState(false);

  const handleCreateShortcut = async () => {
    setCreatingShortcut(true);
    try {
      await settingsApi.createDesktopShortcut();
      toast.success(t("settings.createDesktopShortcutSuccess"));
    } catch (error) {
      console.error("Failed to create desktop shortcut:", error);
      toast.error(t("settings.createDesktopShortcutFailed"));
    } finally {
      setCreatingShortcut(false);
    }
  };

  return (
    <section className="space-y-4">
      <div className="flex items-center gap-2 pb-2 border-b border-border/40">
        <AppWindow className="h-4 w-4 text-primary" />
        <h3 className="text-sm font-medium">{t("settings.windowBehavior")}</h3>
      </div>

      <div className="space-y-3">
        <ToggleRow
          icon={<Power className="h-4 w-4 text-orange-500" />}
          title={t("settings.launchOnStartup")}
          description={t("settings.launchOnStartupDescription")}
          checked={!!settings.launchOnStartup}
          onCheckedChange={(value) => onChange({ launchOnStartup: value })}
        />

        <AnimatePresence initial={false}>
          {settings.launchOnStartup && (
            <motion.div
              key="silent-startup"
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 10 }}
              transition={{ duration: 0.3 }}
            >
              <ToggleRow
                icon={<EyeOff className="h-4 w-4 text-green-500" />}
                title={t("settings.silentStartup")}
                description={t("settings.silentStartupDescription")}
                checked={!!settings.silentStartup}
                onCheckedChange={(value) => onChange({ silentStartup: value })}
              />
            </motion.div>
          )}
        </AnimatePresence>

        
        
        <ToggleRow
          icon={<AppWindow className="h-4 w-4 text-blue-500" />}
          title={t("settings.minimizeToTray")}
          description={t("settings.minimizeToTrayDescription")}
          checked={settings.minimizeToTrayOnClose}
          onCheckedChange={(value) =>
            onChange({ minimizeToTrayOnClose: value })
          }
        />

        {isLinux() && (
          <ToggleRow
            icon={<AppWindow className="h-4 w-4 text-amber-500" />}
            title={t("settings.useAppWindowControls")}
            description={t("settings.useAppWindowControlsDescription")}
            checked={!!settings.useAppWindowControls}
            onCheckedChange={(value) =>
              onChange({ useAppWindowControls: value })
            }
          />
        )}

        <div className="flex items-center justify-between gap-4 rounded-xl border border-border bg-card/50 p-4 transition-colors hover:bg-muted/50">
          <div className="flex items-center gap-3">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-background ring-1 ring-border">
              <MonitorDown className="h-4 w-4 text-sky-500" />
            </div>
            <div className="space-y-1">
              <p className="text-sm font-medium leading-none">
                {t("settings.createDesktopShortcut")}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("settings.createDesktopShortcutDescription")}
              </p>
            </div>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-8 shrink-0 text-xs"
            disabled={creatingShortcut}
            onClick={handleCreateShortcut}
          >
            {t("settings.createDesktopShortcut")}
          </Button>
        </div>
      </div>
    </section>
  );
}
