fn panic_hook(panic_info: &std::panic::PanicHookInfo) {
  // Technically, it's "Ferris throwing up their hands", but oh well...
  eprintln!(
    r#"
                                  -
                         -   --- ---  ---  =-
                        ---------------------
                    -----------------------------
  --    -       #-------------------------------- ---       %    --
 ----  ---       -----------------------------------       ---  ----
 ----- ---   -------------------------------------------   ---%-----
 ---------    -----------------------------------------    ---------
  --------  ----------------------------------------------  -------
      *--   --------------- @@   ----- @@  ---------------  ---
        --- --------------- @@   ----- @@   ------------------
         +-----------------      -----      ----------------
         --------------------  ---   ---:------------::-----
          -----::::::------------      ---------::::::-----
           ----- ::-    :::::::::::::::::::::      :::----
             ----  :                              ::  ---
              ---                                    ---
                --                                   --


                      S A D   F E R R I S   : (


Oh no... Your Ferron web server just crashed...
"#
  );

  let payload = panic_info.payload_as_str();
  eprintln!(
    "{} (at {})",
    payload.unwrap_or("<unknown crash>"),
    panic_info.location().unwrap_or(std::panic::Location::caller())
  );

  eprintln!();
  eprintln!("Backtrace:");

  let backtrace = backtrace::Backtrace::new();
  for frame in backtrace.frames() {
    let symbols = frame.symbols();
    if symbols.is_empty() {
      eprintln!("  at ({:?})", frame.ip());
    } else {
      for symbol in symbols {
        let src_line = symbol
          .filename()
          .and_then(|f| symbol.lineno().map(|l| format!("{}:{}", f.display(), l)));
        eprintln!(
          "  at {}{}",
          symbol.name().map(|n| n.to_string()).unwrap_or("<unknown>".to_string()),
          src_line.map(|l| format!(" ({})", l)).unwrap_or_default()
        );
      }
    }
  }

  eprintln!();
  eprintln!("If you believe it's a bug, please report it at https://github.com/ferronweb/ferron/issues/new");
  eprintln!(
    "Also, consider sharing the backtrace above, and the version information (you can get it by running `ferron -V`)."
  )
}

/// Installs a panic hook
pub fn install_panic_hook() {
  if !shadow_rs::is_debug() {
    std::panic::set_hook(Box::new(panic_hook));
  }
}
