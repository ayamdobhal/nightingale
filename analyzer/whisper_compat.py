"""PyTorch / device compatibility helpers for Nightingale analyzer."""

import importlib
import os
import sys
import time

# Prevent cwd or stray directories from shadowing the real torch package.
# Remove any path entry that contains a bare "torch" directory with a _C
# sub-directory (i.e. a PyTorch source checkout rather than an installed package).
_clean_paths = []
for _p in sys.path:
    _torch_candidate = os.path.join(_p, "torch", "_C")
    if os.path.isdir(_torch_candidate) and not os.path.isfile(
        os.path.join(_p, "torch", "_C" + (".pyd" if os.name == "nt" else ".so"))
    ):
        print(
            f"[whisper_compat] Removing shadowing path from sys.path: {_p}",
            file=sys.stderr,
            flush=True,
        )
        continue
    _clean_paths.append(_p)
sys.path[:] = _clean_paths

# On Windows, torch can partially initialise if a prior process was killed mid-
# CUDA-init, leaving driver locks / DLL state.  Retry the import a few times.
_MAX_IMPORT_RETRIES = 3
torch = None
for _attempt in range(_MAX_IMPORT_RETRIES):
    try:
        if "torch" in sys.modules:
            del sys.modules["torch"]
        import torch as _torch_mod

        # Sanity-check the module actually loaded fully
        if not hasattr(_torch_mod, "load"):
            raise AttributeError("torch imported but missing 'load' — partial init")
        torch = _torch_mod
        break
    except (AttributeError, ImportError, OSError) as exc:
        if _attempt < _MAX_IMPORT_RETRIES - 1:
            print(
                f"[whisper_compat] torch import failed (attempt {_attempt + 1}): {exc}, retrying...",
                file=sys.stderr,
                flush=True,
            )
            time.sleep(1)
        else:
            raise

_original_torch_load = torch.load


def _patched_torch_load(*args, **kwargs):
    kwargs["weights_only"] = False
    return _original_torch_load(*args, **kwargs)


torch.load = _patched_torch_load


def progress(pct: int, msg: str):
    print(f"[nightingale:PROGRESS:{pct}] {msg}", flush=True)


def detect_device() -> str:
    if torch.cuda.is_available():
        return "cuda"
    if torch.backends.mps.is_available():
        return "mps"
    return "cpu"


def align_device_for(device: str) -> str:
    return "cpu" if device == "mps" else device


def compute_type_for(device: str) -> str:
    return "float16" if device == "cuda" else "float32"


def is_oom(err):
    lower = str(err).lower()
    return "out of memory" in lower or "outofmemoryerror" in lower


def free_gpu():
    import gc
    gc.collect()
    try:
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except Exception:
        pass


def align_with_fallback(raw_segments, audio, language, device, pre_align_cleanup=None):
    """Run whisperx.align with OOM fallback: retry after cleanup, then CPU."""
    import whisperx

    align_model = None
    try:
        print(f"[nightingale:LOG] Loading align model for language='{language}' on device='{device}'", flush=True)
        align_model, metadata = whisperx.load_align_model(language_code=language, device=device)
        result = whisperx.align(raw_segments, align_model, metadata, audio, device)
        del align_model
        return result
    except Exception as e:
        if not is_oom(e):
            raise
        if align_model is not None:
            del align_model
            align_model = None
            free_gpu()

        if pre_align_cleanup:
            print(f"[nightingale:LOG] Alignment OOM, freeing whisper model and retrying on {device}", flush=True)
            pre_align_cleanup()
            try:
                align_model, metadata = whisperx.load_align_model(language_code=language, device=device)
                result = whisperx.align(raw_segments, align_model, metadata, audio, device)
                del align_model
                return result
            except Exception as e2:
                if not is_oom(e2):
                    raise
                if align_model is not None:
                    del align_model
                    align_model = None

        print(f"[nightingale:LOG] Alignment OOM, falling back to CPU", flush=True)
        free_gpu()
        align_model, metadata = whisperx.load_align_model(language_code=language, device="cpu")
        result = whisperx.align(raw_segments, align_model, metadata, audio, "cpu")
        del align_model
        return result
