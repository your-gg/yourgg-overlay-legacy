[![Npm][npm-badge]][npm-url]
[![Crates.io][crates-badge]][crates-url]
[![Documentation][docs-badge]][docs-url]

[npm-badge]: https://img.shields.io/npm/v/@asdf-overlay/core.svg
[npm-url]: https://www.npmjs.com/package/@asdf-overlay/core
[crates-badge]: https://img.shields.io/crates/v/asdf-overlay.svg
[crates-url]: https://crates.io/crates/asdf-overlay
[docs-badge]: https://docs.rs/asdf-overlay/badge.svg     
[docs-url]: https://docs.rs/asdf-overlay

# Asdf Overlay
Blazingly fast™ Windows Overlay library

[Documentation](https://storycraft.github.io/asdf-overlay/)

## 기존 Electron 롤 앱에서 사용하기

`@your-gg/yourgg-overlay`는 Electron main process에서 롤 프로세스를 자동
감지하고, 게임이 시작되면 렌더링 DLL을 attach한 뒤 별도의 offscreen
`BrowserWindow`를 게임 화면에 합성한다. 게임이 재시작되면 자동으로 다시
attach한다.

### 요구사항

- Windows
- Electron `^40.1.0`
- `@your-gg/yourgg-core`와 `@your-gg/yourgg-overlay`의 동일 릴리스 버전
- Electron main process에서만 초기화

GitHub Packages에서 설치한다면 앱의 `.npmrc`에 registry를 설정한다.

```ini
@your-gg:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=${NODE_AUTH_TOKEN}
```

```bash
pnpm add @your-gg/yourgg-core @your-gg/yourgg-overlay
```

이 저장소를 직접 연결해 개발할 때는 앱과 이 저장소를 같은 pnpm workspace에
두고 `workspace:*`로 참조할 수 있다.

### Electron main에 연결

오버레이 UI의 URL은 기존 Electron renderer의 별도 route 또는 별도 HTML
entry를 사용하면 된다. 아래 코드는 롤만 감지한다.

```typescript
import { app } from 'electron';
import { startGameOverlay } from '@your-gg/yourgg-overlay';

await app.whenReady();

const overlays = await startGameOverlay('league', {
  url: process.env.OVERLAY_URL ?? 'app://overlay/league',
  width: 520,
  height: 64,
  browserWindow: {
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      // preload: overlayPreloadPath,
    },
  },
});

overlays.events.on('attached', ({ game, process }) => {
  console.log(`${game} overlay attached: ${process.pid}`);
});

overlays.events.on('error', (game, error) => {
  console.error(`${game} overlay failed`, error);
});

app.on('before-quit', () => {
  void overlays.stop();
});
```

롤 프로세스에 attach된 세션은 게임 메모리에서 현재 증강 선택지를 읽어
`game.lol.augment.choices` 이벤트로 전달한다. 패널이 닫히면
`game.lol.augment.error`에 `hud_map_not_found`가 전달된다. 로컬 플레이어가
보유한 증강은 `game.lol.augment.owned`로 전달된다.

```typescript
overlays.events.on('attached', (session) => {
  session.overlay.event.on('game.lol.augment.choices', (choices) => {
    if (choices.mode === 'mayhem') {
      console.log(choices.cards);
    }
  });
  session.overlay.event.on('game.lol.augment.owned', (owned) => {
    console.log(owned.internalNames);
  });
});
```

`startGameOverlay()`는 롤이 실행 중이 아니어도 실패하지 않고 프로세스를 계속
감시한다. import만으로는 프로세스 감시나 DLL attach가 시작되지 않는다.

기본 위치는 우측 상단 18px 여백이다. 위치를 바꾸려면 `placement`를 넘긴다.

```typescript
import { length, percent } from '@your-gg/yourgg-core';

const overlays = await startGameOverlay('league', {
  url: 'app://overlay/league',
  placement: {
    x: percent(0.5),
    y: percent(0),
    anchorX: percent(0.5),
    anchorY: percent(0),
    margin: {
      top: length(24),
    },
  },
});
```

오버레이가 마우스와 키보드를 받아야 할 때만 입력 차단을 명시적으로 켠다.
기본값은 `false`다.

```typescript
overlays.events.on('attached', (session) => {
  if (session.game === 'league') {
    void session.setInteractive(true);
  }
});
```

`setInteractive(true)`는 게임 입력을 차단하므로 단순 정보 표시 UI에서는
호출하지 않는 편이 맞다.

### 발로란트와 롤 동시 지원

하나의 앱에서 두 게임을 모두 감시하려면 게임별 UI URL을 넘긴다. 각 게임은
독립 PID, `Overlay`, `BrowserWindow` 세션을 사용하므로 한 게임의 종료가 다른
게임에 영향을 주지 않는다.

```typescript
import { startGameOverlays } from '@your-gg/yourgg-overlay';

const overlays = await startGameOverlays({
  games: {
    league: {
      url: 'app://overlay/league',
      width: 520,
      height: 64,
    },
    valorant: {
      url: 'app://overlay/valorant',
      width: 520,
      height: 64,
    },
  },
  scanIntervalMs: 3_000,
});
```

### electron-vite / Rollup

네이티브 addon을 번들 안에 넣지 않도록 core 패키지를 external로 유지한다.

```typescript
export default defineConfig({
  main: {
    build: {
      rollupOptions: {
        external: ['@your-gg/yourgg-core'],
      },
    },
  },
});
```

### electron-builder / ASAR

`.node`, `.dll`, `.exe`는 ASAR 내부에서 직접 로드할 수 없다. core 패키지를
통째로 unpack한다.

```json
{
  "build": {
    "asarUnpack": [
      "node_modules/@your-gg/yourgg-core/**"
    ]
  }
}
```

라이브러리가 런타임 경로의 `app.asar`를 `app.asar.unpacked`로 보정하지만,
실제 파일을 unpack하는 작업은 앱 패키저 설정이 담당한다.

### 운영 주의사항

현재 backend는 대상 게임 프로세스에 렌더링 DLL을 주입한다. 저장소 테스트에서
서명된 DLL도 Riot Vanguard의 계정 제재를 받은 이력이 있으며 공식 allow-list
경로가 없다. Riot 게임의 프로덕션 배포에는 안전한 방식이 아니다. 실제 사용자
배포본은 외부 topmost 투명창과 Riot 지원 API 기반 backend로 교체하는 것을
권장한다.

## Used by
[lyrs-url]: https://github.com/organization/lyrs
[tosu-url]: https://github.com/tosuapp/tosu

| Logo | Project | Usage |
| :-----: | ----- | ----- |
| [![Lyrs logo](.github/images/lyrs-logo.png)][lyrs-url] | [Lyrs][lyrs-url] | Ingame lyrics overlay
| [![Tosu logo](.github/images/tosu-logo.png)][tosu-url] | [Tosu][tosu-url] | Ingame overlay

## Sponsorship
[sign-path-io-url]: https://signpath.io/
[sign-path-foundation-url]: https://signpath.org/

| Logo | Description |
| :-----: | ----- |
| [![SignPath logo](.github/images/signpath-logo.png)][sign-path-io-url] | Free code signing provided by [SignPath.io][sign-path-io-url], certificate by [SignPath Foundation][sign-path-foundation-url] |

## Example
Examples are located in `examples` directory.

### Node
Run
```bash
pnpm build && pnpm --filter ingame-browser start all
```

Use `valorant`, `league`, or `all` as the final argument.

https://github.com/user-attachments/assets/d7f0db58-cb11-437f-9990-50d095c7c575

### Rust
1. Run
```bash
pnpm build && cargo run -p noise-rectangle <pid>
```
Glitching squares appear and disappear on target process

https://github.com/user-attachments/assets/069d1cc1-f95d-4a44-899c-7f538c0f5a69

2. Run
```bash
pnpm build && cargo run -p input-capture <pid>
```
It will listen and block inputs from target process until process exit

## License
This project is dual licensed under MIT or Apache-2.0 License
