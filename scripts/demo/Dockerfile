# A machine that can photograph grove.
#
# grove's previews are drawn by herdr as kitty graphics — they never appear in
# the escape-sequence stream, so the asciinema/agg route that works for ordinary
# TUIs would capture the tree beside an empty rectangle. The only way to get a
# real picture is to render one: a real kitty on a real (virtual) X display, with
# herdr inside it, and a screenshot of the result.

# ---- build grove from the working tree ------------------------------------
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

# ---- the darkroom ----------------------------------------------------------
FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        kitty \
        xvfb x11-utils imagemagick \
        libgl1-mesa-dri libglx-mesa0 \
        ca-certificates curl unzip \
        ffmpeg poppler-utils \
        python3-minimal \
        fontconfig \
    && rm -rf /var/lib/apt/lists/*

# Nerd Font, or the tree draws its icons as tofu.
RUN mkdir -p /usr/share/fonts/truetype/nerd \
    && curl -fsSL -o /tmp/font.zip \
        https://github.com/ryanoasis/nerd-fonts/releases/download/v3.4.0/JetBrainsMono.zip \
    && unzip -qo /tmp/font.zip -d /usr/share/fonts/truetype/nerd \
        'JetBrainsMonoNerdFont-Regular.ttf' 'JetBrainsMonoNerdFont-Bold.ttf' \
    && rm /tmp/font.zip \
    && fc-cache -f

# Debian ships ImageMagick with PDF writing disabled — a sensible default when
# processing untrusted input, but the fixture's PDF is generated here, from
# content written three lines further down, inside a throwaway container.
RUN sed -i 's|rights="none" pattern="PDF"|rights="read\|write" pattern="PDF"|' \
        /etc/ImageMagick-6/policy.xml

# herdr itself — grove is a herdr program, so the demo has to be a herdr session.
RUN curl -fsSL https://herdr.dev/install.sh | HERDR_INSTALL_DIR=/usr/local/bin sh

COPY --from=build /src/target/release/grove /usr/local/bin/grove
COPY scripts/demo/kitty.conf /opt/demo/kitty.conf
COPY scripts/demo/make-fixture.sh /opt/demo/make-fixture.sh
COPY scripts/demo/record.sh /opt/demo/record.sh
RUN chmod +x /opt/demo/*.sh

# Software GL: there is no card behind Xvfb, and kitty wants OpenGL 3.3.
ENV LIBGL_ALWAYS_SOFTWARE=1 \
    DISPLAY=:99 \
    TERM=xterm-kitty

ENTRYPOINT ["/opt/demo/record.sh"]
