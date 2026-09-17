import { useEffect, useRef } from "react";
import { ShieldCheck } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import type { ManagedAuthProvider } from "@/lib/api";
import { ProviderIcon } from "@/components/ProviderIcon";
import { XaiOAuthSection } from "@/components/providers/forms/XaiOAuthSection";

interface AuthCenterPanelProps {
  authScrollTarget?: ManagedAuthProvider | null;
}

export function AuthCenterPanel({ authScrollTarget }: AuthCenterPanelProps) {
  const { t } = useTranslation();
  const xaiOauthSectionRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!authScrollTarget) return;

    const sectionRef = xaiOauthSectionRef;

    const frame = requestAnimationFrame(() => {
      const prefersReducedMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)",
      ).matches;

      sectionRef.current?.scrollIntoView({
        behavior: prefersReducedMotion ? "auto" : "smooth",
        block: "start",
      });
    });

    return () => cancelAnimationFrame(frame);
  }, [authScrollTarget]);

  return (
    <div className="space-y-6">
      <section className="rounded-xl border border-border/60 bg-card/60 p-6">
        <div className="flex items-start justify-between gap-4">
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <ShieldCheck className="h-5 w-5 text-primary" />
              <h3 className="text-base font-semibold">
                {t("settings.authCenter.title", {
                  defaultValue: "OAuth 认证中心",
                })}
              </h3>
            </div>
            <p className="text-sm text-muted-foreground">
              {t("settings.authCenter.description", {
                defaultValue: "管理 xAI / Grok 账号。",
              })}
            </p>
          </div>
          <Badge variant="secondary">
            {t("settings.authCenter.beta", { defaultValue: "Beta" })}
          </Badge>
        </div>
      </section>

      <section
        ref={xaiOauthSectionRef}
        className="scroll-mt-4 rounded-xl border border-border/60 bg-card/60 p-6"
      >
        <div className="mb-4 flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-muted">
            <ProviderIcon icon="xai" name="xAI" size={20} />
          </div>
          <div>
            <h4 className="font-medium">xAI (Grok OAuth)</h4>
            <p className="text-sm text-muted-foreground">
              {t("settings.authCenter.xaiOauthDescription", {
                defaultValue: "管理 xAI / Grok 账号",
              })}
            </p>
          </div>
        </div>

        <XaiOAuthSection />
      </section>
    </div>
  );
}
