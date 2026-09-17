import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";

/** 下拉候选项：只需 id；ownedBy 用于分组（缺省归 "Other"）。
 *  兼容通用 FetchedModel 与 OMP 原生目录条目等更宽的来源。 */
export interface ModelDropdownItem {
  id: string;
  ownedBy?: string | null;
}

export function ModelDropdown({
  models,
  onSelect,
}: {
  models: ModelDropdownItem[];
  onSelect: (id: string) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  // Group models by vendor; missing ownedBy falls back to "Other"
  const grouped: Record<string, ModelDropdownItem[]> = {};
  for (const model of models) {
    const vendor = model.ownedBy || "Other";
    if (!grouped[vendor]) grouped[vendor] = [];
    grouped[vendor].push(model);
  }
  const vendors = Object.keys(grouped).sort();

  return (
    <Popover modal open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="icon"
          className="shrink-0"
          aria-label={t("providerForm.searchModelAriaLabel", {
            defaultValue: "Select model",
          })}
        >
          <ChevronDown className="h-4 w-4" />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        sideOffset={4}
        collisionPadding={8}
        className="z-[200] w-72 p-0"
      >
        <Command
          label={t("providerForm.searchModelPlaceholder", {
            defaultValue: "Search models...",
          })}
        >
          <CommandInput
            placeholder={t("providerForm.searchModelPlaceholder", {
              defaultValue: "Search models...",
            })}
          />
          <CommandList className="max-h-64">
            <CommandEmpty>
              {t("providerForm.searchModelEmpty", {
                defaultValue: "No matching models.",
              })}
            </CommandEmpty>
            {vendors.map((vendor) => (
              <CommandGroup key={vendor} heading={vendor}>
                {grouped[vendor].map((m) => (
                  <CommandItem
                    key={m.id}
                    value={m.id}
                    // Expose the vendor name as a keyword so models can also be
                    // fuzzy-matched by vendor, not just by model id.
                    keywords={[m.ownedBy || "Other"]}
                    onSelect={() => {
                      onSelect(m.id);
                      setOpen(false);
                    }}
                  >
                    {m.id}
                  </CommandItem>
                ))}
              </CommandGroup>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
