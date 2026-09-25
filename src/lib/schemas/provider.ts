import { z } from "zod";
import i18n from "@/i18n";

/**
 * 解析 JSON 语法错误，提取位置信息。消息在校验期经 i18n 动态取词
 * （zod 的静态 message 会在模块加载时固化语言，因此全部走 superRefine）。
 */
function parseJsonError(error: unknown): string {
  if (!(error instanceof SyntaxError)) {
    return i18n.t("providerForm.jsonError.invalid");
  }

  const message = error.message;

  // 提取位置信息：Chrome/V8: "Unexpected token ... in JSON at position 123"
  const positionMatch = message.match(/at position (\d+)/i);
  if (positionMatch) {
    const position = parseInt(positionMatch[1], 10);
    return i18n.t("providerForm.jsonError.atPosition", {
      message: message.split(" in JSON")[0],
      position,
    });
  }

  // Firefox: "JSON.parse: unexpected character at line 1 column 23"
  const lineColumnMatch = message.match(/line (\d+) column (\d+)/i);
  if (lineColumnMatch) {
    const line = lineColumnMatch[1];
    const column = lineColumnMatch[2];
    return i18n.t("providerForm.jsonError.atLineColumn", { line, column });
  }

  // 通用情况：原样透出引擎消息（英文），前缀本地化——引擎文本不适合机器翻译拼装
  return i18n.t("providerForm.jsonError.generic", { message });
}

export const providerSchema = z.object({
  name: z.string(), // 必填校验移至 handleSubmit 中用 toast 提示
  websiteUrl: z
    .string()
    .optional()
    .or(z.literal(""))
    .superRefine((value, ctx) => {
      if (value && !/^https?:\/\/.+/.test(value.trim())) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: i18n.t("providerForm.urlInvalid"),
        });
      }
    }),
  notes: z.string().optional(),
  settingsConfig: z
    .string()
    .superRefine((value, ctx) => {
      if (!value.trim()) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: i18n.t("providerForm.configRequired"),
        });
        return;
      }
      try {
        JSON.parse(value);
      } catch (error) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: parseJsonError(error),
        });
      }
    }),
  // 图标配置
  icon: z.string().optional(),
  iconColor: z.string().optional(),
});

export type ProviderFormData = z.infer<typeof providerSchema>;
