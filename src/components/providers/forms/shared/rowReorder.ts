/**
 * 表单内小列表的拖动排序纯函数。
 *
 * 行的稳定 id（uuid）与可编辑字段解耦——模型 ID 在编辑中途可能为空或重复，
 * 不能直接当 SortableContext 的 items / 拖动 id 用。
 */

/** 从「行 id → 内容 id」的对齐数组里算出内容数组的重排结果。 */
export function reorderAligned<T>(rowIds: string[], items: T[], activeRowId: string, overRowId: string): T[] {
  if (activeRowId === overRowId) return items;
  const from = rowIds.indexOf(activeRowId);
  const to = rowIds.indexOf(overRowId);
  if (from === -1 || to === -1) return items;
  const next = [...items];
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved as T);
  return next;
}

/** 同步行 id 列表：不足补新 id（新增行），多余截断（删除行），已有位置保持不变。 */
export function syncRowIds(existing: string[], count: number, makeId: () => string): string[] {
  const next = existing.slice(0, count);
  while (next.length < count) {
    next.push(makeId());
  }
  return next;
}
