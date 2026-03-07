FROM golang:1.24-bookworm AS builder
WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -o /bot ./cmd/bot/

FROM gcr.io/distroless/static-debian12
COPY --from=builder /bot /bot
CMD ["/bot"]
