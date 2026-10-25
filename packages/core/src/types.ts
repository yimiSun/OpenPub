export interface PublishOptions {
  title: string;
  markdown: string;
  tags?: string[];
  coverImage?: string;
  isDraftOnly: boolean;
}

export interface SyncResult {
  success: boolean;
  platform: string;
  draftUrl?: string;
  errorMsg?: string;
}

export interface PlatformAdapter {
  readonly id: string;
  readonly name: string;
  readonly env: ("browser" | "api")[];

  checkAuth(): Promise<boolean>;

  preprocess?(options: PublishOptions): Promise<PublishOptions>;

  uploadImages(images: string[], context: unknown): Promise<string[]>;

  sync(options: PublishOptions, context: unknown): Promise<SyncResult>;
}