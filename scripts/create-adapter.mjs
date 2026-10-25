import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");

const name = process.argv[2];
if (!name) {
  console.error("Usage: node create-adapter.js <platform-name>");
  process.exit(1);
}

const kebab = name.toLowerCase().replace(/[^a-z0-9-]/g, "-");
const pascal = kebab.replace(/(^|-)([a-z0-9])/g, (_, _s, c) => c.toUpperCase());

const adapterDir = path.join(ROOT, "packages", "adapters", kebab);

if (fs.existsSync(adapterDir)) {
  console.error(`Adapter "${kebab}" already exists at ${adapterDir}`);
  process.exit(1);
}

fs.mkdirSync(path.join(adapterDir, "src"), { recursive: true });

fs.writeFileSync(
  path.join(adapterDir, "package.json"),
  JSON.stringify(
    {
      name: `@openpub/adapter-${kebab}`,
      version: "0.0.0",
      private: false,
      main: "dist/index.js",
      module: "dist/index.js",
      types: "dist/index.d.ts",
      exports: {
        ".": {
          types: "./dist/index.d.ts",
          import: "./dist/index.js",
        },
      },
      files: ["dist"],
      scripts: {
        build: "tsc -p tsconfig.json",
        typecheck: "tsc --noEmit",
        lint: "eslint src/",
      },
      dependencies: {
        "@openpub/core": "workspace:*",
      },
      devDependencies: {
        typescript: "^5.7.0",
        "@typescript-eslint/parser": "^8.0.0",
        "@typescript-eslint/eslint-plugin": "^8.0.0",
        eslint: "^9.0.0",
      },
    },
    null,
    2,
  ) + "\n",
);

fs.writeFileSync(
  path.join(adapterDir, "tsconfig.json"),
  JSON.stringify(
    {
      extends: "../../../tsconfig.json",
      compilerOptions: {
        rootDir: "src",
        outDir: "dist",
        composite: true,
      },
      include: ["src"],
    },
    null,
    2,
  ) + "\n",
);

fs.writeFileSync(
  path.join(adapterDir, "src", "adapter.ts"),
  `import type { PlatformAdapter, PublishOptions, SyncResult } from "@openpub/core";

export class ${pascal}Adapter implements PlatformAdapter {
  readonly id = "${kebab}";
  readonly name = "${name}";
  readonly env = ["api"] as const;

  async checkAuth(): Promise<boolean> {
    throw new Error("Not implemented");
  }

  async preprocess?(options: PublishOptions): Promise<PublishOptions> {
    return options;
  }

  async uploadImages(images: string[], _context: unknown): Promise<string[]> {
    return images;
  }

  async sync(options: PublishOptions, _context: unknown): Promise<SyncResult> {
    return {
      success: false,
      platform: this.id,
      errorMsg: "Not implemented",
    };
  }
}
`,
);

fs.writeFileSync(
  path.join(adapterDir, "src", "index.ts"),
  `export { ${pascal}Adapter } from "./adapter";\n`,
);

const workspacePath = path.join(ROOT, "pnpm-workspace.yaml");
let workspaceContent = fs.readFileSync(workspacePath, "utf-8");

if (!workspaceContent.includes("packages/adapters/*")) {
  const lines = workspaceContent.split("\n");
  const packagesIndex = lines.findIndex((l) => l.trim().startsWith("- ") && l.includes("packages/"));
  if (packagesIndex !== -1) {
    lines.splice(packagesIndex + 1, 0, '  - "packages/adapters/*"');
  }
  workspaceContent = lines.join("\n");
  fs.writeFileSync(workspacePath, workspaceContent);
}

console.log(`Adapter "${kebab}" created at ${adapterDir}`);
console.log(`Run "pnpm install" to link the new package.`);