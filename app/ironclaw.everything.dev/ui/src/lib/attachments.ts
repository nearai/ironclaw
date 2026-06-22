interface DownloadFileResponse {
  contentBase64: string;
  mimeType: string;
  filename: string;
  sizeBytes: number;
}

interface AttachmentLimits {
  accept: string[];
  maxCount: number;
  maxFileBytes: number;
  maxTotalBytes: number;
}

interface StagedAttachment {
  id: string;
  filename: string;
  mimeType: string;
  sizeBytes: number;
  sizeLabel: string;
  dataBase64: string;
  kind: "image" | "audio" | "document";
}

export type { AttachmentLimits, StagedAttachment };

export function arrayBufferToBase64(buffer: ArrayBuffer): string {
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const chunkSize = 8192;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.slice(i, i + chunkSize));
  }
  return btoa(binary);
}

export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const value = bytes / 1024 ** i;
  return `${value.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

const MIME_KIND: Record<string, "image" | "audio" | "document"> = {
  "image/": "image",
  "audio/": "audio",
};

export function inferKind(mimeType: string): "image" | "audio" | "document" {
  for (const [prefix, kind] of Object.entries(MIME_KIND)) {
    if (mimeType.startsWith(prefix)) return kind;
  }
  return "document";
}

function wildcardMatch(pattern: string, value: string): boolean {
  if (pattern === "*/*") return true;
  if (pattern.endsWith("/*")) {
    return value.startsWith(pattern.slice(0, -1));
  }
  return pattern === value;
}

function extMatch(ext: string, accept: string): boolean {
  const clean = ext.startsWith(".") ? ext : `.${ext}`;
  return accept.toLowerCase() === clean.toLowerCase();
}

export function isAcceptedFile(file: File, acceptList: string[]): boolean {
  if (!acceptList || acceptList.length === 0) return true;
  for (const pattern of acceptList) {
    if (pattern.startsWith(".")) {
      const fileExt = "." + file.name.split(".").pop()?.toLowerCase();
      if (extMatch(fileExt, pattern)) return true;
    }
    if (wildcardMatch(pattern, file.type)) return true;
  }
  return false;
}

export async function stageFiles(
  files: File[],
  limits: AttachmentLimits,
  existing: StagedAttachment[],
): Promise<{ staged: StagedAttachment[]; errors: string[] }> {
  const errors: string[] = [];
  const staged: StagedAttachment[] = [];

  const totalAfter = existing.length + files.length;
  if (totalAfter > limits.maxCount) {
    const slot = limits.maxCount - existing.length;
    errors.push(
      `Cannot attach more than ${limits.maxCount} file(s). ${slot > 0 ? `You can add ${slot} more.` : "Remove some first."}`,
    );
    if (slot <= 0) return { staged, errors };
    files = files.slice(0, slot);
  }

  const existingBytes = existing.reduce((s, a) => s + a.sizeBytes, 0);

  for (const file of files) {
    if (!isAcceptedFile(file, limits.accept)) {
      errors.push(`"${file.name}" type (${file.type}) is not accepted`);
      continue;
    }
    if (file.size > limits.maxFileBytes) {
      errors.push(
        `"${file.name}" exceeds the maximum file size of ${formatBytes(limits.maxFileBytes)}`,
      );
      continue;
    }
    if (
      existingBytes + staged.reduce((s, a) => s + a.sizeBytes, 0) + file.size >
      limits.maxTotalBytes
    ) {
      errors.push(
        `"${file.name}" would exceed the total attachment size limit of ${formatBytes(limits.maxTotalBytes)}`,
      );
      continue;
    }

    const base64 = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        const comma = result.indexOf(",");
        resolve(comma >= 0 ? result.slice(comma + 1) : result);
      };
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(file);
    });

    staged.push({
      id: crypto.randomUUID(),
      filename: file.name,
      mimeType: file.type || "application/octet-stream",
      sizeBytes: file.size,
      sizeLabel: formatBytes(file.size),
      dataBase64: base64,
      kind: inferKind(file.type || "application/octet-stream"),
    });
  }

  return { staged, errors };
}

export function downloadFile(response: DownloadFileResponse): void {
  const binaryStr = atob(response.contentBase64);
  const bytes = new Uint8Array(binaryStr.length);
  for (let i = 0; i < binaryStr.length; i++) {
    bytes[i] = binaryStr.charCodeAt(i);
  }
  const blob = new Blob([bytes], { type: response.mimeType });
  const blobUrl = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = blobUrl;
  a.download = response.filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(blobUrl);
}
