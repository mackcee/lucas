# Bundled font and native runtime notices

- `egui-fonts/`: original notices from the `epaint_default_fonts` **0.36.2**
  crates.io package, `fonts/`. These fonts are embedded in the Windows UI.
- `ONNX-Runtime-*`: original LICENSE and ThirdPartyNotices.txt from
  <https://github.com/microsoft/onnxruntime/tree/v1.31.0>. ort **2.0.0-rc.14**
  supplies the native 1.31.0 static library at build time. Its pinned Windows
  archive hash is
  `f99aaef23f12986abb05924c944d8433521263ad756b462df762a5a982c98405`.
- `DirectML-*`: original LICENSE.txt and ThirdPartyNotices.txt from the
  **Microsoft.AI.DirectML 1.15.4** NuGet package:
  <https://www.nuget.org/packages/Microsoft.AI.DirectML/1.15.4>.
  The archive's x64 DirectML.dll was byte-compared with ort's supplied DLL.
  This is a Windows redistributable with Microsoft's own terms, not LUCAS's MIT
  option. LUCAS selects only ONNX Runtime's CPU execution provider.
- CI also copies Microsoft's app-local Visual C++ redistributable DLLs from the
  GitHub Windows runner's Visual Studio `VC/Redist/MSVC/.../x64/Microsoft.VC*.CRT`.
  These runtime binaries remain Microsoft software, not LUCAS source.

The original notice wording is retained; DirectML line endings and trailing
whitespace are normalized. Update these notices when
upgrading their corresponding pinned dependency or native runtime.
