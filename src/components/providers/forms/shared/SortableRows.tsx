/**
 * 表单内小列表的拖动排序容器（@dnd-kit）。
 *
 * 与 ProviderList 的供应商拖排同一套参数（closestCenter、PointerSensor
 * distance:8、KeyboardSensor），但做成通用小组件供模型列表编辑器复用：
 * - `SortableRows`：DndContext + SortableContext + onDragEnd 适配；
 * - `SortableRow`：单行包装，render-prop 暴露手柄所需的 attributes/listeners；
 * - `RowDragHandle`：拖动手柄按钮（GripVertical），listeners 只挂手柄，
 *   行内输入框/下拉照常可点。
 */

import { type ReactNode } from "react";
import { GripVertical } from "lucide-react";
import { useTranslation } from "react-i18next";
import { DndContext, closestCenter, type DragEndEvent } from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
  sortableKeyboardCoordinates,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  PointerSensor,
  KeyboardSensor,
  useSensor,
  useSensors,
  type DraggableAttributes,
  type DraggableSyntheticListeners,
} from "@dnd-kit/core";
import { cn } from "@/lib/utils";

export interface RowDragProps {
  attributes: DraggableAttributes;
  listeners: DraggableSyntheticListeners;
  isDragging: boolean;
}

interface SortableRowsProps {
  /** 与行一一对应的稳定 id（uuid），不是内容 id */
  rowIds: string[];
  onReorder: (activeRowId: string, overRowId: string) => void;
  children: ReactNode;
}

export function SortableRows({ rowIds, onReorder, children }: SortableRowsProps) {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    onReorder(String(active.id), String(over.id));
  };

  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
      <SortableContext items={rowIds} strategy={verticalListSortingStrategy}>
        {children}
      </SortableContext>
    </DndContext>
  );
}

interface SortableRowProps {
  rowId: string;
  children: (dragProps: RowDragProps) => ReactNode;
}

export function SortableRow({ rowId, children }: SortableRowProps) {
  const { setNodeRef, attributes, listeners, transform, transition, isDragging } =
    useSortable({ id: rowId });

  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
    >
      {children({ attributes, listeners, isDragging })}
    </div>
  );
}

export function RowDragHandle({ attributes, listeners, isDragging }: RowDragProps) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      className={cn(
        "-ml-1 flex-shrink-0 cursor-grab touch-none p-1 text-muted-foreground/50 transition-colors hover:text-muted-foreground active:cursor-grabbing",
        isDragging && "cursor-grabbing text-muted-foreground",
      )}
      aria-label={t("provider.dragHandle", { defaultValue: "拖拽排序" })}
      {...attributes}
      {...listeners}
    >
      <GripVertical className="h-4 w-4" />
    </button>
  );
}
