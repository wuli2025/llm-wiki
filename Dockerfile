# ════════════════════════════════════════════════════════════════
# Polaris · Docker 化镜像（方案 A：保留 Rust 引擎，axum 替代 Tauri 外壳）
#   阶段1 web      —— 构建 Vue3 前端 → dist/
#   阶段2 server   —— 构建 polaris-server（复用同一份 Rust 引擎，不含 Tauri）
#   阶段3 runtime  —— node-slim + 预装 claude CLI，托管前端 + 跑 HTTP/WS 服务
#
# 构建：docker build -t polaris-web .
# 运行：见 docker-compose.yml
# ════════════════════════════════════════════════════════════════

# ── 阶段1：构建前端 ──────────────────────────────────────────────
FROM node:20-slim AS web
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY index.html vite.config.ts tsconfig.json tsconfig.node.json ./
COPY public ./public
COPY src ./src
RUN npm run build      # → /app/dist

# ── 阶段2：构建 Rust server 二进制 ───────────────────────────────
FROM rust:1-slim-bookworm AS server
# ring(经 ureq/rustls) 需要 C 编译器；其余解析库均为纯 Rust。
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build

# 2a) 依赖缓存层：先只拷清单 + crates 源 + 空占位 src，预编译全部第三方依赖。
#     之后改业务代码不会重编 axum/tokio 等重型依赖 → Windows 更新后 Docker 快速重建。
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/build.rs ./src-tauri/
COPY src-tauri/crates ./src-tauri/crates
RUN mkdir -p src-tauri/src/bin \
    && echo 'fn main(){}' > src-tauri/src/bin/polaris-server.rs \
    && echo '' > src-tauri/src/main.rs \
    && echo '' > src-tauri/src/lib.rs \
    && cargo build --profile release-fast \
        --manifest-path src-tauri/Cargo.toml \
        --bin polaris-server --no-default-features --features server \
    ; rm -rf src-tauri/src

# 2b) 真实源码层：拷源码 + 资源 + assets(feishu/wecom 的 include_str!)，编出 polaris-server。
COPY src-tauri/src ./src-tauri/src
COPY src-tauri/assets ./src-tauri/assets
COPY src-tauri/resources ./src-tauri/resources
# 触碰 mtime 确保 cargo 重编 polaris-app crate 本体（而非缓存的空壳）。
RUN touch src-tauri/src/main.rs src-tauri/src/lib.rs \
    && cargo build --profile release-fast \
        --manifest-path src-tauri/Cargo.toml \
        --bin polaris-server --no-default-features --features server \
    && cp src-tauri/target/release-fast/polaris-server /usr/local/bin/polaris-server

# ── 阶段3：运行时 ────────────────────────────────────────────────
FROM node:20-slim AS runtime
# claude CLI 跑 Bash/脚本工具需要：bash、git、python3(pptx/xlsx 等技能)、ripgrep、ca 证书。
RUN apt-get update && apt-get install -y --no-install-recommends \
        bash git ca-certificates curl python3 python3-pip python3-venv ripgrep \
        tini gosu \
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g @anthropic-ai/claude-code \
    && npm cache clean --force

# ── 渲染栈(可选 flavor)——Polaris Forge 跨平台 PRD §05：容器「零安装」=渲染栈打进镜像 ──
#   POLARIS_RENDER=0 → polaris:slim   现状(聊天/KB/网站生成，网站本就不需渲染栈)
#   POLARIS_RENDER=1 → polaris:full   +chromium(截图)+fonts-noto-cjk(防豆腐块)+ffmpeg(出视频)
# 构建 full：docker build --build-arg POLARIS_RENDER=1 -t polaris-web:full .
# CJK 字体是「最隐蔽必踩」坑：缺了 deck 截图全是 □□□，preflight 会用 fc-list 探测并亮红灯。
ARG POLARIS_RENDER=0
RUN if [ "$POLARIS_RENDER" = "1" ]; then \
        apt-get update && apt-get install -y --no-install-recommends \
            chromium fonts-noto-cjk fonts-noto-color-emoji ffmpeg \
        && rm -rf /var/lib/apt/lists/* ; \
    else \
        echo "[build] POLARIS_RENDER=0 → slim 镜像(无渲染栈)" ; \
    fi
# 让引擎 preflight 能定位浏览器/编码器(slim 下这些路径不存在，preflight 会据此降级)。
ENV POLARIS_CHROMIUM=/usr/bin/chromium \
    POLARIS_FFMPEG=ffmpeg \
    POLARIS_RENDER_FLAVOR=${POLARIS_RENDER}

# 引擎二进制 + 前端静态 + 资源种子
COPY --from=server /usr/local/bin/polaris-server /usr/local/bin/polaris-server
COPY --from=web    /app/dist /srv/web
COPY src-tauri/resources /app/resources

ENV HOME=/root \
    POLARIS_RESOURCE_DIR=/app/resources \
    POLARIS_WEB_DIR=/srv/web \
    POLARIS_PORT=8080 \
    # claude headless 默认非交互；让其在容器里直接用环境变量鉴权
    CI=1

# 入口脚本：tini 作 PID 1（镜像内自带，回收 claude spawn 的子进程僵尸，
# 不再依赖 compose `init: true` 在群晖 Container Manager 下是否生效）；
# 脚本按 PUID/PGID 决定 root / 非 root 运行。sed 去 CR 防 Windows 换行致 exec 失败。
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN sed -i 's/\r$//' /usr/local/bin/docker-entrypoint.sh \
    && chmod +x /usr/local/bin/docker-entrypoint.sh

EXPOSE 8080
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]
