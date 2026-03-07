package main

import (
	"context"
	"os"
	"os/signal"
	"syscall"

	"go.uber.org/zap"

	"meet-bot/internal/bot"
	"meet-bot/internal/config"
	"meet-bot/internal/web"
)

func main() {
	log, _ := zap.NewProduction()
	defer log.Sync() //nolint:errcheck

	cfg, err := config.Load()
	if err != nil {
		log.Fatal("config error", zap.Error(err))
	}

	b := bot.New(cfg, log)

	ctx, stop := signal.NotifyContext(context.Background(),
		os.Interrupt, syscall.SIGTERM)
	defer stop()

	// Start web UI server
	srv := web.NewServer(cfg, b.Agent(), b.VexaClient(), log)
	b.SetBroadcast(srv.BroadcastTranscript)

	go func() {
		if err := srv.Start(ctx); err != nil {
			log.Error("web server error", zap.Error(err))
		}
	}()

	if err := b.Run(ctx); err != nil {
		log.Fatal("bot error", zap.Error(err))
	}
}
