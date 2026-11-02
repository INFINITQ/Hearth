# Running models with Hearth

Hearth runs open models on your PC through llama.cpp's `llama-server`. You install llama.cpp once,
download model files (`.gguf`), and tell Hearth which folder they're in. Hearth never downloads
anything for you.

**Your PC:** RTX 5070 (12 GB VRAM) · 32 GB RAM · Ryzen 5 9600X · Windows 11.

The best models for this PC are _mixture-of-experts_ (MoE) models: only a few billion of their
weights work on each token, so they stay fast even though most of the file sits in system RAM
instead of on the graphics card. Model files up to about 22 GB run well.

_Checked against Hugging Face and the llama.cpp releases on 2 November 2026. Models change monthly;
check the repo's Files tab for exact names and sizes before downloading._

---

## Step 1: install llama.cpp (once)

1. Open <https://github.com/ggml-org/llama.cpp/releases>.
2. Find the newest release named with a **build number** such as `b11960`. (The release marked
   "Latest", `v0.8.0` at the time of writing, only contains a text file.)
3. Under **Assets**, download both:
   - `llama-b11960-bin-win-cuda-13.4-x64.zip` (≈165 MB): the programs
   - `cudart-llama-bin-win-cuda-13.4-x64.zip` (≈424 MB): NVIDIA's CUDA runtime files

   Use the **CUDA 13.4** builds. The CUDA 12.4 ones don't support RTX 50-series cards. They need
   NVIDIA driver 580 or newer (yours is 617).

4. Right-click each zip → **Properties** → tick **Unblock** → OK. Otherwise SmartScreen may block
   the programs. Then extract **both** zips into the same folder, for example `C:\Tools\llama.cpp`.
5. Add that folder to your PATH. Run this once in PowerShell, then open a new terminal:

   ```powershell
   [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path', 'User') + ';C:\Tools\llama.cpp', 'User')
   ```

6. Check it:

   ```powershell
   llama-server --version
   llama-server --list-devices
   ```

   The second command must list `CUDA0: NVIDIA GeForce RTX 5070`. If it lists only the CPU, the
   cudart files are missing from the folder.

**Updating later:** download the newer pair of zips and extract them over the same folder.
(`winget install llama.cpp` also works, but it installs the Vulkan build, which is slower on
NVIDIA cards.)

---

## Step 2: choose models

| Use                  | Model           | Repo → file                                                              | Size    | Why                                                                                                                                  |
| -------------------- | --------------- | ------------------------------------------------------------------------ | ------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| **Coding** (default) | Qwen3.6-35B-A3B | `unsloth/Qwen3.6-35B-A3B-GGUF` → `Qwen3.6-35B-A3B-UD-IQ4_XS.gguf`        | 17.7 GB | The strongest coding model that runs well here (73.4% on SWE-bench Verified, vendor-reported). 262K context. Also a good chat model. |
|                      |                 | …or `Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf`                                    | 22.4 GB | Slightly better quality; leaves less RAM free.                                                                                       |
| **Chat** (default)   | Gemma 4 26B-A4B | `unsloth/gemma-4-26B-A4B-it-GGUF` → `gemma-4-26B-A4B-it-UD-Q4_K_XL.gguf` | 17 GB   | Strong all-rounder with good writing. 256K context.                                                                                  |
|                      |                 | …or `gemma-4-26B-A4B-it-UD-IQ4_XS.gguf`                                  | 13.6 GB | Smaller, a little faster.                                                                                                            |
| **Fast** all-rounder | gpt-oss-20b     | `ggml-org/gpt-oss-20b-GGUF` → `gpt-oss-20b-MXFP4.gguf`                   | 12.1 GB | Almost entirely on the graphics card, so the fastest. Reasoning effort low / medium / high. Text only.                               |
| **Light**            | Gemma 4 12B     | `unsloth/gemma-4-12b-it-GGUF` → its `UD-Q4_K_XL` file                    | ≈8 GB   | Fits fully on the GPU: instant replies and very fast reading of long prompts. Smaller brain.                                         |

**Images:** Qwen3.6 and Gemma 4 can read images. Put the repo's `mmproj-F16.gguf` (0.9–1.2 GB) in
the same folder as the model. Keep only **one** mmproj file per folder.

**One big model at a time:** with 32 GB of RAM, only one of the 13–22 GB models fits at once. If
Chat and Code use different models, Hearth swaps them when you switch modes (10–30 seconds). Using
Qwen3.6 for both avoids swapping.

**Speed:** MoE models are reported at about 30–50 tokens per second on 12 GB cards. Dense models
in the 27–31B range, such as Qwen3.8-27B and Gemma 4 31B, are smarter but don't fit in 12 GB and
run at around 10 tokens per second. Skip those for now.

### Reading the file names

- **35B-A3B:** 35 billion weights in total, about 3 billion active per token. The active size sets
  the speed; the total size sets the memory needed.
- **Quantization** (how much the weights are compressed), from best to smallest:
  - **Q8_0:** near-lossless, big.
  - **Q6_K** and **Q5_K_M:** very good.
  - **Q4_K_M** and **UD-Q4_K_XL:** the usual sweet spot.
  - **IQ4_XS:** a bit smaller and nearly as good.
  - **Q3_K_\*:** noticeably weaker.

  **UD** means Unsloth Dynamic: the most important layers are kept at higher precision.

- **mmproj-\*.gguf:** the image reader for a model. Only needed if you send images.

---

## Step 3: download

Make one folder per model, for example `D:\Models\Qwen3.6-35B-A3B\`.

**Option A, browser:** open the repo on huggingface.co → **Files and versions** → click the file →
**Download**. Move it (and the mmproj file, if you want images) into the model's folder.

**Option B, command line** (resumes interrupted downloads):

```powershell
py -m pip install -U huggingface_hub
hf download unsloth/Qwen3.6-35B-A3B-GGUF Qwen3.6-35B-A3B-UD-IQ4_XS.gguf mmproj-F16.gguf --local-dir D:\Models\Qwen3.6-35B-A3B
hf download unsloth/gemma-4-26B-A4B-it-GGUF gemma-4-26B-A4B-it-UD-Q4_K_XL.gguf mmproj-F16.gguf --local-dir D:\Models\gemma-4-26B-A4B
hf download ggml-org/gpt-oss-20b-GGUF gpt-oss-20b-MXFP4.gguf --local-dir D:\Models\gpt-oss-20b
```

If `hf` isn't recognized after installing, open a new terminal; if it still isn't, the browser
method works just as well.

**Option C, LM Studio:** if you like browsing models in an app, LM Studio's Discover tab downloads
GGUF files to `%USERPROFILE%\.lmstudio\models`. Hearth can use that folder too.

**Download safety:**

- Only download `.gguf` files. They hold weights, not code.
- Never run `.exe`, `.bat` or `.py` files from a model repo, and avoid old `.bin` / `.pt` files,
  which can carry code.
- Prefer well-known uploaders (unsloth, ggml-org, bartowski, the model makers).
- If a finished download is much smaller than the size listed on the site, it's incomplete:
  download it again.

---

## Step 4: use them in Hearth

> The Models page that does this for you arrives with milestone M5. Until then, run llama-server
> yourself (below) and connect Hearth to it.

1. **Settings → Models:** Hearth finds `llama-server` on your PATH, or you browse to
   `llama-server.exe`. It shows your GPU as `CUDA0`.
2. Add your models folder (`D:\Models`). Each model appears with its size and what it can do
   (tools, images, thinking).
3. Pick the **Chat** model and the **Coding** model.
4. The first load reads the whole file from disk (10–60 seconds). Hearth uses a 32K context for
   chat and 64K for coding, and lets llama.cpp decide what goes on the GPU.

### Running llama-server by hand

```powershell
llama-server -m D:\Models\Qwen3.6-35B-A3B\Qwen3.6-35B-A3B-UD-IQ4_XS.gguf -c 65536 -np 1 --jinja --host 127.0.0.1 --port 8080
```

Add `--mmproj D:\Models\Qwen3.6-35B-A3B\mmproj-F16.gguf` for images. Then, in Hearth, add an
OpenAI-compatible connection to `http://127.0.0.1:8080/v1`.

---

## Uncensored models

These are community-made versions of open models with the refusal behaviour removed.

- **Abliteration** finds the model's "refusal direction" and removes it from the weights.
- **Heretic** (a tool by p-e-w) automates abliteration and measures how far the result drifts from
  the original, as a KL divergence (lower is closer). Heretic builds usually lose the least
  quality.

**Trade-offs:**

- There is a small quality loss; for good Heretic builds it's often barely measurable.
- The model is less cautious in general, not only about what you intended.
- Tool calling, which the coding agent depends on, is less tested on these builds. Keep the
  official model as your default coding model and try the uncensored one next to it.

| Based on        | Uncensored repo → file                                                                                | Size      | Measured by the uploader                                                                                      |
| --------------- | ----------------------------------------------------------------------------------------------------- | --------- | ------------------------------------------------------------------------------------------------------------- |
| Qwen3.6-35B-A3B | `llmfan46/Qwen3.6-35B-A3B-uncensored-heretic-GGUF` → `Qwen3.6-35B-A3B-uncensored-heretic-Q3_K_L.gguf` | 18.2 GB   | Refuses 10/100 test prompts (original: 83/100). KL 0.0015, MMLU 83.3 vs 83.7                                  |
|                 | …or `Qwen3.6-35B-A3B-uncensored-heretic-Q4_K_M.gguf`                                                  | 21.2 GB   | Its mmproj file (for images) is listed in the Files tab                                                       |
| Gemma 4 26B-A4B | `nohurry/gemma-4-26B-A4B-it-heretic-GUFF` → `gemma-4-26b-a4b-it-heretic.q4_k_m.gguf`                  | 16.9 GB   | 11/100 refusals, KL 0.0499 (Heretic build by coder3101). Images: `gemma-4-26B-A4B-it-heretic-mmproj.f16.gguf` |
|                 | …or `gemma-4-26b-a4b-it-heretic.iq4_xs.gguf`                                                          | 15 GB     |                                                                                                               |
| gpt-oss-20b     | `p-e-w/gpt-oss-20b-heretic-ara-v3` → pick a GGUF from **Quantizations** on its page                   | ≈12–13 GB | 3/100 refusals (original: 98/100), KL 0.0554. Made by Heretic's author                                        |

```powershell
hf download llmfan46/Qwen3.6-35B-A3B-uncensored-heretic-GGUF Qwen3.6-35B-A3B-uncensored-heretic-Q3_K_L.gguf --local-dir D:\Models\Qwen3.6-35B-A3B-heretic
hf download nohurry/gemma-4-26B-A4B-it-heretic-GUFF gemma-4-26b-a4b-it-heretic.q4_k_m.gguf gemma-4-26B-A4B-it-heretic-mmproj.f16.gguf --local-dir D:\Models\gemma-4-26B-A4B-heretic
```

**Finding more:** search Hugging Face for "heretic GGUF" or "abliterated GGUF". Good model cards
report refusals and KL divergence. The same download-safety rules apply.

---

## Which programs run these models

|                                     | llama.cpp (Hearth's default)                                    | LM Studio                               | Ollama                                                                                                       |
| ----------------------------------- | --------------------------------------------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| All the models above                | Yes                                                             | Yes (download them in its Discover tab) | Yes, via `ollama pull hf.co/<repo>:<quant>` (Gemma 4 uncensored also as `huihui_ai/gemma-4-abliterated:26b`) |
| Speed of MoE models on a 12 GB card | Best: expert weights kept in RAM, the rest placed automatically | Good: has an expert-offload option      | Slower: splits the model by layer                                                                            |
| With Hearth                         | Managed by Hearth                                               | As an OpenAI-compatible connection      | As an OpenAI-compatible connection                                                                           |

---

## Tuning

- **Context length** is how much text the model can keep in view. Longer context needs more
  memory and makes long prompts slower to read. 32K–64K is a good balance.
- **Thinking:** Qwen3.6 and Gemma 4 can think before they answer. That's better for hard problems
  and slower for simple chats. Hearth lets you switch it per chat.
- **If it's slow:**
  - close GPU-heavy apps (games, video in a browser);
  - make sure `--list-devices` shows `CUDA0`;
  - prefer an IQ4_XS file.
- **Advanced:** each model in Hearth gets an "extra arguments" field.
  - `--n-cpu-moe 30` forces how many layers' experts stay in RAM.
  - Speed-up files for speculative decoding are supported by newer llama.cpp builds: Gemma 4's
    `mtp-…gguf` and gpt-oss's `eagle3-…gguf`.

---

## Troubleshooting

| Problem                                              | Fix                                                                                                                         |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `--list-devices` doesn't show `CUDA0`                | You extracted the Vulkan or CPU build, or the cudart zip isn't in the same folder. Updating the NVIDIA driver can help too. |
| "failed to allocate" / "out of memory" while loading | Lower the context, use a smaller file (IQ4_XS or Q3), or close other apps.                                                  |
| Very slow (under 5 tokens per second)                | The model is running on the CPU (see the first row), or it's a dense model too big for the GPU.                             |
| Garbled or repeating output                          | Update llama.cpp, re-download the file (it may be corrupt), or reset the sampling settings.                                 |
| Tool calls show up as plain text                     | Update llama.cpp (tool-call support improves often). Hearth also tries to recover them.                                     |
| Images are ignored                                   | The model's mmproj file isn't in its folder, or isn't loaded.                                                               |

---

## Sources

- [llama.cpp releases](https://github.com/ggml-org/llama.cpp/releases) and [server docs](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)
- [unsloth/Qwen3.6-35B-A3B-GGUF](https://huggingface.co/unsloth/Qwen3.6-35B-A3B-GGUF), [unsloth/gemma-4-26B-A4B-it-GGUF](https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF), [ggml-org/gpt-oss-20b-GGUF](https://huggingface.co/ggml-org/gpt-oss-20b-GGUF), [unsloth/gemma-4-12b-it-GGUF](https://huggingface.co/unsloth/gemma-4-12b-it-GGUF)
- [llmfan46/Qwen3.6-35B-A3B-uncensored-heretic-GGUF](https://huggingface.co/llmfan46/Qwen3.6-35B-A3B-uncensored-heretic-GGUF), [nohurry/gemma-4-26B-A4B-it-heretic-GUFF](https://huggingface.co/nohurry/gemma-4-26B-A4B-it-heretic-GUFF), [p-e-w/gpt-oss-20b-heretic-ara-v3](https://huggingface.co/p-e-w/gpt-oss-20b-heretic-ara-v3)
- [Qwen3.6 benchmarks](https://www.siliconflow.com/blog/qwen3.6-series-now-on-siliconflow-flagship-level-coding-in-compact-models), [Kilo: best local coding models](https://blog.kilo.ai/p/the-best-local-coding-models-for), [MoE offload on 12 GB cards](https://openclawdc.com/blog/llama-cpp-moe-offload-flags-explained/), [abliterated models by VRAM](https://locallyuncensored.com/blog/abliterated-models-guide.html)
