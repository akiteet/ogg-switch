/**
 * OGG Switch Type Definitions
 * 
 * Core types for Agent switching between Grok Build and Oh My Pi
 */

/**
 * Agent type identifier
 * - grokbuild: Grok Build (xAI Grok CLI)
 * - omp: Oh My Pi (OMP)
 */
export type AppId = "grokbuild" | "omp";

/**
 * Visible apps configuration
 */
export interface VisibleApps {
  grokbuild: boolean;
  omp: boolean;
}

/**
 * Default visible apps (both enabled)
 */
export const DEFAULT_VISIBLE_APPS: VisibleApps = {
  grokbuild: true,
  omp: true,
};

// Re-export other types
export * from "./env";
export * from "./icon";
export * from "./omo";
export * from "./proxy";
export * from "./subscription";
export * from "./usage";
