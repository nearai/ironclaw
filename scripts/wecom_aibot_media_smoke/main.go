package main

import (
	"bytes"
	"crypto/md5"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"os"
	"path/filepath"
	"strings"
	"time"

	aibot "github.com/go-sphere/wecom-aibot-go-sdk/aibot"
)

const defaultAuthTimeout = 15 * time.Second

type config struct {
	botID       string
	botSecret   string
	target      string
	imagePath   string
	wsURL       string
	authTimeout time.Duration
	sendText    string
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "FAIL: %v\n", err)
		os.Exit(1)
	}
}

func run() error {
	cfg, err := loadConfig()
	if err != nil {
		return err
	}

	image, filename, cleanup, err := readImage(cfg.imagePath)
	if err != nil {
		return err
	}
	defer cleanup()

	authenticated := make(chan struct{})
	errorsCh := make(chan error, 4)

	client := aibot.NewWSClient(aibot.WSClientOptions{
		BotID:          cfg.botID,
		Secret:         cfg.botSecret,
		WSURL:          cfg.wsURL,
		RequestTimeout: int(15 * time.Second / time.Millisecond),
	})
	client.OnConnected(func() {
		fmt.Println("connected")
	})
	client.OnAuthenticated(func() {
		fmt.Println("authenticated")
		close(authenticated)
	})
	client.OnDisconnected(func(reason string) {
		if reason == "" {
			reason = "unknown"
		}
		errorsCh <- fmt.Errorf("websocket disconnected before send completed: %s", reason)
	})
	client.OnError(func(err error) {
		if err != nil {
			errorsCh <- err
		}
	})

	client.Connect()
	defer client.Disconnect()

	select {
	case <-authenticated:
	case err := <-errorsCh:
		return err
	case <-time.After(cfg.authTimeout):
		return fmt.Errorf("timed out waiting for bot authentication after %s", cfg.authTimeout)
	}

	if cfg.sendText != "" {
		if _, err := client.SendMarkdown(cfg.target, cfg.sendText); err != nil {
			return fmt.Errorf("send markdown probe: %w", err)
		}
		fmt.Println("sent markdown probe")
	}

	uploaded, err := uploadMediaCompat(client, image, aibot.UploadMediaOptions{
		Type:     aibot.WeComMediaType("image"),
		Filename: filename,
	})
	if err != nil {
		return fmt.Errorf("upload image media: %w", err)
	}
	if uploaded == nil || strings.TrimSpace(uploaded.MediaID) == "" {
		return errors.New("upload image media returned empty media_id")
	}
	fmt.Printf("uploaded image: type=%s media_id=%s\n", uploaded.Type, uploaded.MediaID)

	frame, err := client.SendMediaMessage(cfg.target, aibot.WeComMediaType("image"), uploaded.MediaID, nil)
	if err != nil {
		return fmt.Errorf("send image media: %w", err)
	}
	if frame != nil && frame.ErrCode != 0 {
		return fmt.Errorf("send image media returned errcode=%d errmsg=%s", frame.ErrCode, frame.ErrMsg)
	}

	fmt.Printf("sent image to %s\n", cfg.target)
	return nil
}

type mediaUploadResult struct {
	Type    aibot.WeComMediaType `json:"type"`
	MediaID string               `json:"media_id"`
}

func uploadMediaCompat(client *aibot.WSClient, fileBuffer []byte, options aibot.UploadMediaOptions) (*mediaUploadResult, error) {
	totalSize := len(fileBuffer)
	const chunkSize = 512 * 1024
	totalChunks := (totalSize + chunkSize - 1) / chunkSize
	if totalChunks == 0 {
		totalChunks = 1
	}
	if totalChunks > 100 {
		return nil, fmt.Errorf("file too large: %d chunks exceeds maximum of 100 chunks", totalChunks)
	}

	hash := md5.Sum(fileBuffer)
	initFrame, err := replyWithGeneratedReqID(client, aibot.WsCmd.UPLOAD_MEDIA_INIT, aibot.UploadMediaInitBody{
		Type:        options.Type,
		Filename:    options.Filename,
		TotalSize:   totalSize,
		TotalChunks: totalChunks,
		MD5:         hex.EncodeToString(hash[:]),
	})
	if err != nil {
		return nil, fmt.Errorf("upload init failed: %w", err)
	}

	var initResp aibot.UploadMediaInitResult
	if err := json.Unmarshal(initFrame.Body, &initResp); err != nil {
		return nil, fmt.Errorf("upload init response parse failed: %w", err)
	}
	if strings.TrimSpace(initResp.UploadID) == "" {
		return nil, errors.New("upload init failed: no upload_id returned")
	}
	fmt.Printf("upload init: upload_id=%s chunks=%d\n", initResp.UploadID, totalChunks)

	for i := 0; i < totalChunks; i++ {
		start := i * chunkSize
		end := start + chunkSize
		if end > totalSize {
			end = totalSize
		}
		chunk := fileBuffer[start:end]
		_, err := replyWithGeneratedReqID(client, aibot.WsCmd.UPLOAD_MEDIA_CHUNK, aibot.UploadMediaChunkBody{
			UploadID:   initResp.UploadID,
			ChunkIndex: i,
			Base64Data: base64.StdEncoding.EncodeToString(chunk),
		})
		if err != nil {
			return nil, fmt.Errorf("upload chunk %d failed: %w", i, err)
		}
	}
	fmt.Printf("uploaded chunks: %d\n", totalChunks)

	finishFrame, err := replyWithGeneratedReqID(client, aibot.WsCmd.UPLOAD_MEDIA_FINISH, aibot.UploadMediaFinishBody{
		UploadID: initResp.UploadID,
	})
	if err != nil {
		return nil, fmt.Errorf("upload finish failed: %w", err)
	}

	var finishResp mediaUploadResult
	if err := json.Unmarshal(finishFrame.Body, &finishResp); err != nil {
		return nil, fmt.Errorf("upload finish response parse failed: %w", err)
	}
	if strings.TrimSpace(finishResp.MediaID) == "" {
		return nil, errors.New("upload finish failed: no media_id returned")
	}
	return &finishResp, nil
}

func replyWithGeneratedReqID(client *aibot.WSClient, cmd string, body interface{}) (*aibot.WsFrame, error) {
	return client.Reply(&aibot.WsFrame{
		Headers: aibot.WsFrameHeaders{
			ReqID: aibot.GenerateReqId(cmd),
		},
	}, body, cmd)
}

func loadConfig() (config, error) {
	cfg := config{
		botID:       env("WECOM_BOT_ID"),
		botSecret:   env("WECOM_BOT_SECRET"),
		target:      firstNonEmpty(env("WECOM_CHAT_ID"), env("WECOM_TO")),
		imagePath:   env("WECOM_IMAGE_PATH"),
		wsURL:       env("WECOM_WS_URL"),
		authTimeout: defaultAuthTimeout,
		sendText:    env("WECOM_SMOKE_TEXT"),
	}

	if raw := env("WECOM_AUTH_TIMEOUT_SECS"); raw != "" {
		seconds, err := time.ParseDuration(raw + "s")
		if err != nil {
			return config{}, fmt.Errorf("WECOM_AUTH_TIMEOUT_SECS must be a number of seconds: %w", err)
		}
		cfg.authTimeout = seconds
	}

	var missing []string
	if cfg.botID == "" {
		missing = append(missing, "WECOM_BOT_ID")
	}
	if cfg.botSecret == "" {
		missing = append(missing, "WECOM_BOT_SECRET")
	}
	if cfg.target == "" {
		missing = append(missing, "WECOM_CHAT_ID or WECOM_TO")
	}
	if len(missing) > 0 {
		return config{}, fmt.Errorf("missing required env: %s", strings.Join(missing, ", "))
	}

	return cfg, nil
}

func readImage(path string) ([]byte, string, func(), error) {
	if path != "" {
		data, err := os.ReadFile(path)
		if err != nil {
			return nil, "", func() {}, fmt.Errorf("read WECOM_IMAGE_PATH: %w", err)
		}
		return data, filepath.Base(path), func() {}, nil
	}

	data, err := buildFallbackPNG()
	if err != nil {
		return nil, "", func() {}, fmt.Errorf("build fallback png: %w", err)
	}
	tmp, err := os.CreateTemp("", "wecom-aibot-smoke-*.png")
	if err != nil {
		return nil, "", func() {}, fmt.Errorf("create fallback png: %w", err)
	}
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		_ = os.Remove(tmp.Name())
		return nil, "", func() {}, fmt.Errorf("write fallback png: %w", err)
	}
	if err := tmp.Close(); err != nil {
		_ = os.Remove(tmp.Name())
		return nil, "", func() {}, fmt.Errorf("close fallback png: %w", err)
	}

	cleanup := func() {
		_ = os.Remove(tmp.Name())
	}
	return data, filepath.Base(tmp.Name()), cleanup, nil
}

func buildFallbackPNG() ([]byte, error) {
	img := image.NewRGBA(image.Rect(0, 0, 160, 90))
	bg := color.RGBA{R: 28, G: 104, B: 255, A: 255}
	accent := color.RGBA{R: 255, G: 207, B: 72, A: 255}
	white := color.RGBA{R: 255, G: 255, B: 255, A: 255}

	for y := 0; y < 90; y++ {
		for x := 0; x < 160; x++ {
			img.SetRGBA(x, y, bg)
			if x > 104 || y < 18 {
				img.SetRGBA(x, y, accent)
			}
			if x > 26 && x < 134 && y > 38 && y < 52 {
				img.SetRGBA(x, y, white)
			}
		}
	}

	var buf bytes.Buffer
	if err := png.Encode(&buf, img); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

func env(name string) string {
	return strings.TrimSpace(os.Getenv(name))
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}
