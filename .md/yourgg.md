# your-gg/yourgg-overlay 포크 운영 가이드

> `storycraft/asdf-overlay` 포크를 Azure Trusted Signing + GitHub Packages 기반으로 운영하는 방법.
>
> **모든 작업은 `yourgg` 브랜치에서 진행합니다.**

---

## 저장소 · 패키지 구조

| 항목 | 내용 |
| --- | --- |
| upstream | [storycraft/asdf-overlay](https://github.com/storycraft/asdf-overlay) |
| 포크 (monorepo) | [your-gg/yourgg-overlay](https://github.com/your-gg/yourgg-overlay) |
| core 패키지 repo | [your-gg/yourgg-core](https://github.com/your-gg/yourgg-core) |
| 운영 브랜치 | `yourgg` |
| upstream sync 브랜치 | `main` |
| npm 레지스트리 | [npm.pkg.github.com](https://npm.pkg.github.com) |

### npm 패키지

| npm 패키지 | 소스 경로 |
| --- | --- |
| `@your-gg/yourgg-core` | `packages/core` |
| `@your-gg/yourgg-overlay` | `packages/electron` |

Phase 1: `yourgg-overlay` monorepo에서 두 패키지 함께 publish.

### DLL · addon 산출물

| 파일 | 비고 |
| --- | --- |
| `yourgg_overlay-x64.dll` | rebrand |
| `yourgg_overlay-x86.dll` | rebrand |
| `yourgg_overlay-aarch64.dll` | rebrand |
| `addon-x64.node` | upstream과 동일 이름 |
| `addon-aarch64.node` | upstream과 동일 이름 |

Rust crate 이름(`asdf-overlay-dll` 등)은 upstream sync 편의상 유지. xtask 출력 파일명만 변경.

---

## 1. 최초 설정 (한 번만)

### 1-1. yourgg 브랜치 push

```bash
git checkout yourgg
git push -u origin yourgg
```

### 1-2. GitHub Environments

`your-gg/yourgg-overlay` → Settings → Environments

| Environment | 용도 |
| --- | --- |
| `deploy` | 빌드 + Azure 서명 |
| `publish` | npm publish + GitHub Release |

### 1-3. GitHub Secrets

| Secret | 내용 |
| --- | --- |
| `AZURE_TENANT_ID` | Azure 테넌트 ID |
| `AZURE_CLIENT_ID` | 서비스 주체 클라이언트 ID |
| `AZURE_CLIENT_SECRET` | 서비스 주체 시크릿 |
| `AZURE_SIGNING_ENDPOINT` | `https://xxx.codesigning.azure.net/` |
| `AZURE_SIGNING_ACCOUNT` | Trusted Signing 계정명 |
| `AZURE_SIGNING_PROFILE` | 인증서 프로필명 |

제거: `SIGNPATH_API_TOKEN`, `CRATES_IO_TOKEN`

### 1-4. 코드 변경 (적용 완료)

- `@your-gg/yourgg-core`, `@your-gg/yourgg-overlay` rebrand
- `yourgg_overlay-*.dll` rename
- `.github/workflows/deploy.yml`, `publish.yml` Azure + GitHub Packages

### 1-5. 커밋 & 푸시

```bash
git add .
git commit -m "chore: rebrand to your-gg, Azure Trusted Signing + GitHub Packages"
git push origin yourgg
```

---

## 2. 패키지 배포

GitHub Actions → **publish** → Run workflow

```
Branch: yourgg
Bump level: patch / minor / major
Actually publish: false  ← dry-run
Actually publish: true   ← 실제 배포
```

**배포 흐름:**

```
publish.yml
  └─ deploy.yml (pnpm build + artifact upload)
  └─ execute=true → Azure Trusted Signing (.dll + .node)
  └─ artifact download → packages/core
  └─ pnpm ci:publish → GitHub Packages
  └─ GitHub Release (yourgg_overlay-*.dll)
```

---

## 3. upstream 업데이트 (필요할 때만)

```bash
gh repo sync your-gg/yourgg-overlay --source storycraft/asdf-overlay
git checkout yourgg && git merge main
```

**충돌 시 our 버전 유지:**

```
.github/workflows/deploy.yml
.github/workflows/publish.yml
packages/core/package.json
packages/electron/package.json
package.json
xtask/src/main.rs
packages/core/native/src/overlay.rs
.gitignore
@your-gg/* import 경로
```

upstream trailing 필수 아님. 필요한 fix/feature만 cherry-pick해도 됨.

---

## 4. 사용처 프로젝트 설정

### 4-1. .npmrc

```ini
@your-gg:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=${NPM_TOKEN}
```

로컬: GitHub PAT (`read:packages`). CI: `GITHUB_TOKEN`.

### 4-2. 설치

```bash
npm install @your-gg/yourgg-core @your-gg/yourgg-overlay
```

### 4-3. import

```typescript
import { Overlay, defaultDllDir } from '@your-gg/yourgg-core';
import { ElectronOverlaySurface } from '@your-gg/yourgg-overlay/surface';
import { ElectronOverlayInput } from '@your-gg/yourgg-overlay/input';
```

### 4-4. electron-builder

```yaml
asarUnpack:
  - "node_modules/@your-gg/**"
```

---

## deploy.yml 전문

```yaml
name: deploy

on:
  workflow_call:
    inputs:
      sign:
        required: false
        type: boolean
        default: false
    outputs:
      unsigned-artifact-id:
        description: "Unsigned artifact Id"
        value: ${{ jobs.deploy.outputs.unsigned-artifact-id }}
      signed-artifact-id:
        description: "Signed artifact Id"
        value: ${{ jobs.deploy.outputs.signed-artifact-id }}
    secrets:
      AZURE_TENANT_ID:
        required: false
      AZURE_CLIENT_ID:
        required: false
      AZURE_CLIENT_SECRET:
        required: false
      AZURE_SIGNING_ENDPOINT:
        required: false
      AZURE_SIGNING_ACCOUNT:
        required: false
      AZURE_SIGNING_PROFILE:
        required: false
  workflow_dispatch:
    inputs:
      sign:
        type: boolean
        required: false
        description: Sign artifacts
        default: false
  pull_request:
    paths:
      - 'Cargo.toml'
      - 'Cargo.lock'
      - 'package.json'
      - 'pnpm-lock.yaml'
      - 'crates/**/*'
      - 'src/**/*'
      - 'xtask/**/*'

concurrency:
  group: deploy-${{ github.ref_name }}
  cancel-in-progress: true

jobs:
  deploy:
    runs-on: windows-latest
    environment: deploy
    outputs:
      unsigned-artifact-id: ${{ steps.upload-unsigned-artifact.outputs.artifact-id }}
      signed-artifact-id: ${{ steps.upload-signed-artifact.outputs.artifact-id }}
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive

      - name: Setup cd
        run: |
          git config user.name "YOUR.GG Overlay Continuous Deployment"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

      - name: Extract Commit SHA
        id: commit-sha
        run: |
          "short_sha=" + "${{ github.sha }}".SubString(0, 8) >> $env:GITHUB_OUTPUT

      - name: Setup rust
        uses: dtolnay/rust-toolchain@nightly

      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: 'deploy'
          cache-on-failure: true
          cache-all-crates: true

      - name: Setup pnpm
        uses: pnpm/action-setup@v6

      - name: Setup Node.js
        uses: actions/setup-node@v6
        with:
          node-version: 24
          registry-url: 'https://npm.pkg.github.com'
          cache: 'pnpm'

      - name: Install dependencies
        run: pnpm install

      - name: Deploy
        run: npm run ci:deploy

      - name: Upload unsigned artifact
        id: upload-unsigned-artifact
        uses: actions/upload-artifact@v4
        with:
          name: 'yourgg-overlay-artifact-unsigned-${{ steps.commit-sha.outputs.short_sha }}'
          path: |
            packages/core/addon-aarch64.node
            packages/core/addon-x64.node
            packages/core/yourgg_overlay-aarch64.dll
            packages/core/yourgg_overlay-x86.dll
            packages/core/yourgg_overlay-x64.dll

      - name: Download unsigned artifact for signing
        if: ${{ inputs.sign }}
        uses: actions/download-artifact@v4
        with:
          artifact-ids: ${{ steps.upload-unsigned-artifact.outputs.artifact-id }}
          path: 'target/unsigned'
          merge-multiple: true

      - name: Sign artifacts
        if: ${{ inputs.sign }}
        uses: azure/artifact-signing-action@v1
        with:
          azure-tenant-id: ${{ secrets.AZURE_TENANT_ID }}
          azure-client-id: ${{ secrets.AZURE_CLIENT_ID }}
          azure-client-secret: ${{ secrets.AZURE_CLIENT_SECRET }}
          endpoint: ${{ secrets.AZURE_SIGNING_ENDPOINT }}
          signing-account-name: ${{ secrets.AZURE_SIGNING_ACCOUNT }}
          certificate-profile-name: ${{ secrets.AZURE_SIGNING_PROFILE }}
          files-folder: target/unsigned
          files-folder-filter: dll,node
          file-digest: SHA256
          timestamp-rfc3161: http://timestamp.acs.microsoft.com
          timestamp-digest: SHA256

      - name: Upload signed artifact
        id: upload-signed-artifact
        if: ${{ inputs.sign }}
        uses: actions/upload-artifact@v4
        with:
          name: 'yourgg-overlay-artifact-${{ steps.commit-sha.outputs.short_sha }}'
          path: target/unsigned/**/*
```

---

## publish.yml 전문

```yaml
name: publish

concurrency: production

on:
  workflow_dispatch:
    inputs:
      level:
        type: choice
        description: Bump level
        options:
          - major
          - minor
          - patch
      execute:
        type: boolean
        description: Actually publish (not dry run)

jobs:
  release-deploy:
    uses: ./.github/workflows/deploy.yml
    with:
      sign: ${{ inputs.execute == true }}
    secrets:
      AZURE_TENANT_ID: ${{ secrets.AZURE_TENANT_ID }}
      AZURE_CLIENT_ID: ${{ secrets.AZURE_CLIENT_ID }}
      AZURE_CLIENT_SECRET: ${{ secrets.AZURE_CLIENT_SECRET }}
      AZURE_SIGNING_ENDPOINT: ${{ secrets.AZURE_SIGNING_ENDPOINT }}
      AZURE_SIGNING_ACCOUNT: ${{ secrets.AZURE_SIGNING_ACCOUNT }}
      AZURE_SIGNING_PROFILE: ${{ secrets.AZURE_SIGNING_PROFILE }}

  publish:
    runs-on: windows-latest
    environment: publish
    needs: release-deploy
    permissions:
      contents: write
      packages: write
      id-token: write
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive

      - name: Setup cd
        run: |
          git config user.name 'YOUR.GG Overlay Continuous Deployment'
          git config user.email '41898282+github-actions[bot]@users.noreply.github.com'

      - name: Setup pnpm
        uses: pnpm/action-setup@v6

      - name: Setup Node.js
        uses: actions/setup-node@v6
        with:
          node-version: 24
          registry-url: 'https://npm.pkg.github.com'
          scope: '@your-gg'
          cache: 'pnpm'

      - name: Install dependencies
        run: pnpm install

      - name: Bump node package version
        id: bump
        run: |
          npm config set git-tag-version=false
          $ver = pnpm --filter "@your-gg/*" exec npm version ${{ github.event.inputs.level }}
          $ver=$ver[0]
          git commit -a -m "chore: Bump node package version"
          "version=" + $ver.substring(1) >> $env:GITHUB_OUTPUT

      - name: Download artifacts
        uses: actions/download-artifact@v4
        with:
          artifact-ids: ${{ inputs.execute && needs.release-deploy.outputs.signed-artifact-id || needs.release-deploy.outputs.unsigned-artifact-id }}
          path: 'packages/core'
          merge-multiple: true

      - name: Publish node package
        run: pnpm ci:publish ${{ !inputs.execute && '--dry-run' || '' }}
        env:
          NODE_AUTH_TOKEN: ${{ secrets.GITHUB_TOKEN }}

      - name: Release
        if: ${{ inputs.execute }}
        uses: softprops/action-gh-release@v2
        with:
          name: 'YOUR.GG Overlay v${{ steps.bump.outputs.version }}'
          draft: false
          generate_release_notes: true
          make_latest: true
          tag_name: "v${{ steps.bump.outputs.version }}"
          target_commitish: '${{ github.ref }}'
          files: |
            packages/core/yourgg_overlay-aarch64.dll
            packages/core/yourgg_overlay-x86.dll
            packages/core/yourgg_overlay-x64.dll
```
