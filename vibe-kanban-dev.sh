#!/bin/bash
# Vibe Kanban 开发环境启动脚本 (前台模式)
# 使用方法: ./vibe-kanban-dev.sh
# 按 Ctrl+C 停止服务

set -e

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_FILE="/tmp/server.log"
FRONTEND_LOG="/tmp/frontend.log"
NODE_VERSION="v24.13.0"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# 清理函数
cleanup() {
    echo ""
    log_warn "接收到中断信号，正在停止服务..."
    if [ -n "$SERVER_PID" ]; then
        kill $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true
    fi
    if [ -n "$FRONTEND_PID" ]; then
        kill $FRONTEND_PID 2>/dev/null || true
        wait $FRONTEND_PID 2>/dev/null || true
    fi
    log_info "服务已停止"
    exit 0
}

trap cleanup INT TERM

# 检查环境
check_env() {
    log_step "检查环境..."
    
    # 检查 Node
    if [ ! -f "$HOME/.nvm/versions/node/$NODE_VERSION/bin/node" ]; then
        log_error "Node.js $NODE_VERSION 未找到"
        exit 1
    fi
    
    # 检查 Rust
    if ! command -v cargo &> /dev/null; then
        log_error "Rust/Cargo 未安装"
        exit 1
    fi
    
    # 检查 pnpm
    if ! command -v pnpm &> /dev/null; then
        log_error "pnpm 未安装"
        exit 1
    fi
    
    log_info "环境检查通过"
}

# 停止已有服务
stop_existing() {
    log_step "停止已有服务..."
    pkill -f "target/debug/server" 2>/dev/null || true
    pkill -f "vite" 2>/dev/null || true
    sleep 2
}

# 加载 cargo 环境（必须在检查环境之前）
if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi

# 主流程
echo "=================================="
echo " Vibe Kanban 开发环境启动脚本"
echo "=================================="
echo ""

check_env
stop_existing

cd "$PROJECT_DIR"

# 环境变量
export VK_SHARED_API_BASE="https://api.vibekanban.com"
export VK_ALLOWED_ORIGINS="http://localhost:3000"
export PATH="$HOME/.nvm/versions/node/$NODE_VERSION/bin:$PATH"

# 清理日志
rm -f "$LOG_FILE" "$FRONTEND_LOG"

# 启动后端
log_step "启动后端服务..."
cargo run --bin server > "$LOG_FILE" 2>&1 &
SERVER_PID=$!
log_info "后端 PID: $SERVER_PID"

# 等待后端启动
log_info "等待后端启动..."
BACKEND_PORT=""
for i in {1..30}; do
    if grep -q "Main server on" "$LOG_FILE" 2>/dev/null; then
        # 修复：提取 Main server 端口（不是 Preview proxy）
        # 日志格式包含颜色代码: "Main server on :XXXXX, Preview proxy on :YYYYY"
        # 使用 awk 提取 Main server 后面的第一个端口号
        LOG_LINE=$(grep "Main server on" "$LOG_FILE" | head -1)
        BACKEND_PORT=$(echo "$LOG_LINE" | awk -F'Main server on :' '{print $2}' | awk -F'[^0-9]' '{print $1}')
        if [ -n "$BACKEND_PORT" ] && [ "$BACKEND_PORT" -gt 10000 ]; then
            log_info "✅ 后端已启动，API 端口: $BACKEND_PORT"
            break
        fi
    fi
    sleep 1
done

if [ -z "$BACKEND_PORT" ] || [ "$BACKEND_PORT" -lt 10000 ]; then
    log_error "❌ 后端启动失败或端口无效"
    echo "--- 后端日志 ---"
    tail -30 "$LOG_FILE"
    exit 1
fi

# 启动前端
log_step "启动前端服务..."
export BACKEND_PORT=$BACKEND_PORT
cd packages/local-web
pnpm run dev > "$FRONTEND_LOG" 2>&1 &
FRONTEND_PID=$!
log_info "前端 PID: $FRONTEND_PID"

# 等待前端启动
log_info "等待前端启动..."
FRONTEND_PORT=""
for i in {1..20}; do
    if lsof -Pn -iTCP:3000 2>/dev/null | grep -q "node"; then
        FRONTEND_PORT=3000
        log_info "✅ 前端已启动，端口: 3000"
        break
    elif lsof -Pn -iTCP:3001 2>/dev/null | grep -q "node"; then
        FRONTEND_PORT=3001
        log_info "✅ 前端已启动，端口: 3001 (3000 被占用)"
        break
    fi
    sleep 1
done

if [ -z "$FRONTEND_PORT" ]; then
    log_error "❌ 前端启动失败"
    echo "--- 前端日志 ---"
    tail -30 "$FRONTEND_LOG"
    exit 1
fi

# 测试 API
log_step "测试 API 连接..."
for i in {1..10}; do
    if curl -s "http://localhost:$FRONTEND_PORT/api/organizations" > /dev/null 2>&1; then
        log_info "✅ API 连接正常"
        break
    fi
    sleep 1
done

echo ""
echo "=================================="
echo -e "${GREEN}🎉 启动成功!${NC}"
echo "=================================="
echo ""
echo "访问地址:"
echo "  前端: http://localhost:$FRONTEND_PORT/"
echo "  后端: http://localhost:$BACKEND_PORT/"
echo ""
echo "进程信息:"
echo "  后端 PID: $SERVER_PID (端口: $BACKEND_PORT)"
echo "  前端 PID: $FRONTEND_PID (端口: $FRONTEND_PORT)"
echo ""
echo -e "${YELLOW}按 Ctrl+C 停止服务${NC}"
echo "=================================="
echo ""

# 持续打印日志
wait $SERVER_PID $FRONTEND_PID 2>/dev/null || true
