// --- Image Upload ---

function renderImagePreviews() {
  const strip = document.getElementById('image-preview-strip');
  strip.innerHTML = '';
  stagedImages.forEach((img, idx) => {
    const container = document.createElement('div');
    container.className = 'image-preview-container';

    const preview = document.createElement('img');
    preview.className = 'image-preview';
    preview.src = img.dataUrl;
    preview.alt = 'Attached image';

    const removeBtn = document.createElement('button');
    removeBtn.className = 'image-preview-remove';
    removeBtn.textContent = '\u00d7';
    removeBtn.addEventListener('click', () => {
      stagedImages.splice(idx, 1);
      renderImagePreviews();
    });

    container.appendChild(preview);
    container.appendChild(removeBtn);
    strip.appendChild(container);
  });
}

const MAX_IMAGE_SIZE_BYTES = 5 * 1024 * 1024; // 5 MB per image
const MAX_STAGED_IMAGES = 5;

function handleImageFiles(files) {
  Array.from(files).forEach(file => {
    if (!file.type.startsWith('image/')) return;
    if (file.size > MAX_IMAGE_SIZE_BYTES) {
      alert(I18n.t('chat.imageTooBig', { name: file.name, size: (file.size / 1024 / 1024).toFixed(1) }));
      return;
    }
    if (stagedImages.length >= MAX_STAGED_IMAGES) {
      alert(I18n.t('chat.maxImages', { n: MAX_STAGED_IMAGES }));
      return;
    }
    const reader = new FileReader();
    reader.onload = function(e) {
      const dataUrl = e.target.result;
      const commaIdx = dataUrl.indexOf(',');
      const meta = dataUrl.substring(0, commaIdx); // e.g. "data:image/png;base64"
      const base64 = dataUrl.substring(commaIdx + 1);
      const mediaType = meta.replace('data:', '').replace(';base64', '');
      stagedImages.push({ media_type: mediaType, data: base64, dataUrl: dataUrl });
      renderImagePreviews();
    };
    reader.readAsDataURL(file);
  });
}

document.getElementById('attach-btn').addEventListener('click', () => {
  document.getElementById('image-file-input').click();
});

document.getElementById('image-file-input').addEventListener('change', (e) => {
  handleImageFiles(e.target.files);
  e.target.value = '';
});

document.getElementById('chat-input').addEventListener('paste', (e) => {
  const items = (e.clipboardData || e.originalEvent.clipboardData).items;
  for (let i = 0; i < items.length; i++) {
    if (items[i].kind === 'file' && items[i].type.startsWith('image/')) {
      const file = items[i].getAsFile();
      if (file) handleImageFiles([file]);
    }
  }
});

const chatMessagesEl = document.getElementById('chat-messages');
chatMessagesEl.addEventListener('copy', (e) => {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed) return;
  const anchorNode = selection.anchorNode;
  const focusNode = selection.focusNode;
  if (!anchorNode || !focusNode) return;
  if (!chatMessagesEl.contains(anchorNode) || !chatMessagesEl.contains(focusNode)) return;
  const text = selection.toString();
  if (!text || !e.clipboardData) return;
  // Force plain-text clipboard output so dark-theme styling never leaks on paste.
  e.preventDefault();
  e.clipboardData.clearData();
  e.clipboardData.setData('text/plain', text);
});

function createGeneratedImageElement(dataUrl, path, eventId) {
  const card = document.createElement('div');
  card.className = 'generated-image-card';
  if (eventId) {
    card.dataset.imageEventId = eventId;
  }

  if (isSafeGeneratedImageDataUrl(dataUrl)) {
    const img = document.createElement('img');
    img.className = 'generated-image';
    img.src = dataUrl;
    img.alt = 'Generated image';
    card.appendChild(img);
  } else {
    const placeholder = document.createElement('div');
    placeholder.className = 'generated-image-placeholder';
    placeholder.textContent = 'Generated image unavailable in history payload';
    card.appendChild(placeholder);
  }

  if (path) {
    const pathLabel = document.createElement('div');
    pathLabel.className = 'generated-image-path';
    pathLabel.textContent = path;
    card.appendChild(pathLabel);
  }

  return card;
}

function isSafeGeneratedImageDataUrl(dataUrl) {
  return typeof dataUrl === 'string' && /^data:image\//i.test(dataUrl);
}

function hasRenderedGeneratedImage(container, eventId) {
  if (!eventId) return false;
  return Array.from(container.querySelectorAll('.generated-image-card')).some((card) => {
    return card.dataset.imageEventId === eventId;
  });
}

function addGeneratedImage(dataUrl, path, eventId, shouldScroll = true) {
  const container = document.getElementById('chat-messages');
  if (hasRenderedGeneratedImage(container, eventId)) {
    return;
  }
  const card = createGeneratedImageElement(dataUrl, path, eventId);
  container.appendChild(card);
  if (shouldScroll) {
    container.scrollTop = container.scrollHeight;
  }
}

function rememberGeneratedImage(threadId, eventId, dataUrl, path) {
  if (!threadId || !eventId || !isSafeGeneratedImageDataUrl(dataUrl)) return;
  const normalizedPath = path || null;
  let images = generatedImagesByThread.get(threadId);
  if (!images) {
    if (generatedImagesByThread.size >= GENERATED_IMAGE_THREAD_CACHE_CAP) {
      const oldestThreadId = generatedImagesByThread.keys().next().value;
      if (oldestThreadId) {
        generatedImagesByThread.delete(oldestThreadId);
      }
    }
    images = [];
    generatedImagesByThread.set(threadId, images);
  } else {
    // Refresh insertion order so recently viewed/updated threads stay cached.
    generatedImagesByThread.delete(threadId);
    generatedImagesByThread.set(threadId, images);
  }
  if (images.some(img => img.eventId === eventId)) {
    return;
  }
  images.push({ eventId, dataUrl, path: normalizedPath });
  while (images.length > GENERATED_IMAGES_PER_THREAD_CAP) {
    images.shift();
  }
}

function getRememberedGeneratedImage(threadId, eventId) {
  if (!threadId || !eventId) return null;
  const images = generatedImagesByThread.get(threadId);
  if (!images) return null;
  return images.find(img => img.eventId === eventId) || null;
}

function resolveGeneratedImageForRender(threadId, image) {
  const normalizedPath = image.path || null;
  if (image.data_url) {
    return { dataUrl: image.data_url, path: normalizedPath };
  }
  const remembered = getRememberedGeneratedImage(threadId, image.event_id);
  if (remembered) {
    return { dataUrl: remembered.dataUrl, path: remembered.path };
  }
  return { dataUrl: null, path: normalizedPath };
}
