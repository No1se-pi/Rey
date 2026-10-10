"""python -m app.v201 [cli|telegram]"""
from __future__ import annotations

import argparse
import asyncio
import logging


def main() -> None:
    parser = argparse.ArgumentParser(prog='Rey 2.0.1')
    parser.add_argument('channel', choices=['cli', 'telegram', 'voice-ref'], nargs='?', default='cli')
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format='%(levelname)s %(name)s: %(message)s')
    try:
        if args.channel == 'cli':
            from .cli import run_cli
            run_cli()
        elif args.channel == 'telegram':
            from .telegram_bot import run_telegram
            asyncio.run(run_telegram())
        else:
            from .voice_ref import create_reference
            create_reference()
    except ValueError as exc:
        raise SystemExit(f'Configuration error: {exc}') from exc


if __name__ == '__main__':
    main()
