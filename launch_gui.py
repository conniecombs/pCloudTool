#!/usr/bin/env python3
"""
Simple launcher for pCloud Fast Transfer GUI
"""

import sys

try:
    from gui import main
    main()
except ImportError as e:
    print("Error: Missing dependencies!")
    print("\nPlease install required packages:")
    print("  pip install -r requirements.txt")
    print(f"\nDetails: {e}")
    sys.exit(1)
except Exception as e:
    print(f"Error launching GUI: {e}")
    import traceback
    traceback.print_exc()
    sys.exit(1)
