import type { Translator } from "@/i18n";

export function companionError(t: Translator, code: string): string {
  const key = "companion.error." + code;
  const message = t(key);
  if (message !== key) return message;
  const chatKey = "vnchat.error." + code;
  const chatMessage = t(chatKey);
  return chatMessage === chatKey ? t("vnchat.error.unknown") : chatMessage;
}
