# Julia + jlrs environment for fish.
#
# Install:  cp contrib/julia.fish ~/.config/fish/conf.d/julia.fish
#
# Deliberately not wrapped in `status is-interactive`: fish scripts and cargo
# invocations started from non-interactive shells need these too.

# Point this at your Julia directory. A version-independent symlink makes upgrades a
# one-liner:  ln -sfn ~/julia-1.13.4 ~/julia
set -l julia_home ~/julia

if test -d $julia_home
    # Build time: jlrs finds headers and libjulia via `which julia`.
    # -g keeps this session-scoped so this file stays the single source of truth.
    fish_add_path -g $julia_home/bin

    # Build time, optional: makes jlrs independent of PATH order.
    set -gx JLRS_JULIA_DIR $julia_home

    # Run time: the dynamic loader has to find libjulia.so.
    # Not needed for binaries built with JLRS_JULIA_DIR set (README, "Rpath").
    if not contains -- $julia_home/lib $LD_LIBRARY_PATH
        set -gx LD_LIBRARY_PATH $julia_home/lib $LD_LIBRARY_PATH
    end
end
