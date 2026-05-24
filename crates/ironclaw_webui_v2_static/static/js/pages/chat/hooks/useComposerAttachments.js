import { React } from "../../../lib/html.js";

const MAX_FILE_SIZE = 5 * 1024 * 1024;
const MAX_TOTAL_SIZE = 10 * 1024 * 1024;

export function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function readFile(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = reader.result;
      const base64 = dataUrl.split(",")[1];
      resolve({ dataUrl, base64, mime_type: file.type, filename: file.name, size: file.size });
    };
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });
}

export function useComposerAttachments() {
  const [images, setImages] = React.useState([]);
  const [attachments, setAttachments] = React.useState([]);

  const addFiles = React.useCallback(
    async (files) => {
      const newImages = [];
      const newAttachments = [];
      let totalSize = attachments.reduce((sum, item) => sum + item.size, 0);
      totalSize += images.reduce((sum, item) => sum + item.size, 0);

      for (const file of files) {
        if (file.size > MAX_FILE_SIZE) continue;
        if (totalSize + file.size > MAX_TOTAL_SIZE) break;
        totalSize += file.size;

        const info = await readFile(file);
        if (file.type.startsWith("image/")) {
          newImages.push(info);
        } else {
          newAttachments.push(info);
        }
      }

      setImages((prev) => [...prev, ...newImages]);
      setAttachments((prev) => [...prev, ...newAttachments]);
    },
    [attachments, images]
  );

  return {
    images,
    attachments,
    addFiles,
    removeImage: (index) => setImages((prev) => prev.filter((_, idx) => idx !== index)),
    removeAttachment: (index) => setAttachments((prev) => prev.filter((_, idx) => idx !== index)),
    clearAttachments: () => {
      setImages([]);
      setAttachments([]);
    },
  };
}
