export interface ThreadSummary {
  id: string;
  cwd: string;
  folderName: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  userMessages: number;
  agentMessages: number;
  fileSize: number;
  archived: boolean;
  source: string;
  cliVersion: string;
  filePath: string;
}

export interface ThreadListPayload {
  total: number;
  lastScanAt: number;
  items: ThreadSummary[];
}

export interface ThreadExportPayload {
  exported: number;
  directory: string;
  files: string[];
}
