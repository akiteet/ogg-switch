# NOTICE

OGG Switch is a fork of **cc-switch**.

- Project: https://github.com/akiteet/ogg-switch
- Upstream project: https://github.com/farion1231/cc-switch
- Upstream version forked: **v3.20.3**
- Upstream license: MIT
- Upstream copyright: Copyright (c) 2025 Jason Young

The original MIT license text is retained unmodified in [LICENSE](LICENSE), as the license requires.

## What was reused

The application shell, provider-management UI, local-proxy infrastructure and the Grok Build configuration writer originate from cc-switch. This fork extends them into a dual-engine control plane for Grok Build and Oh My Pi: a rebuilt provider-preset catalog, native Oh My Pi (`models.yml` / `config.yml`) integration with semantic role orchestration, session and skill management, usage analytics, and a multi-platform release pipeline. A summary of the changes is in [CHANGELOG.md](CHANGELOG.md).

## What was removed

Documentation, CI configuration, packaging targets and marketing assets belonging to the upstream project (partner logos, screenshots, multi-language routing guides, community templates, release history) were removed because they do not apply to this fork. Their absence does not imply any change to the upstream license.

## Trademarks

"grok", "xAI" and any related marks belong to their respective owners. This project is an independent tool and is not affiliated with or endorsed by xAI or by the cc-switch maintainer.
