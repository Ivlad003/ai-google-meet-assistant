.PHONY: build run dev models docker docker-run test lint tidy

build:
	CGO_ENABLED=1 go build -o ./bin/bot ./cmd/bot/

run: build
	./bin/bot

dev:
	go run ./cmd/bot/

models:
	./scripts/download-models.sh

test:
	go test ./... -v

docker:
	docker compose build

docker-run:
	docker compose up

docker-logs:
	docker compose logs -f meet-bot

lint:
	golangci-lint run ./...

tidy:
	go mod tidy
