export interface PluginItem {
  id: string;
  displayName: string;
  shortDescription: string;
  longDescription: string;
  category: string;
  capabilities: string[];
  version: string;
  authentication: string;
  authenticationLabel: string;
  installation: "available" | "installed";
  installed: boolean;
  installedVersion?: string;
  payloadMode: "usb-preloaded";
  developerName: string;
  brandColor?: string;
  iconUrl?: string;
}

export interface PluginCatalog {
  items: PluginItem[];
  total: number;
  page: number;
  pageSize: number;
  categories: string[];
  payloadMode: "usb-preloaded";
}

export interface PluginInstallResult {
  id: string;
  version: string;
  installed: boolean;
  reinstalled: boolean;
  installPath: string;
  packageSha256: string;
  authentication: string;
  nextAction: string;
}

export interface ApiEnvelope<T> {
  ok: boolean;
  data: T;
  error?: string;
}

export type InstallState =
  | { kind: "idle" }
  | { kind: "installing"; message: string }
  | { kind: "success"; message: string }
  | { kind: "error"; message: string };
