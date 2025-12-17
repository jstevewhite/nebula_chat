import React, { useState, useRef, useEffect } from 'react';
import { ChevronDown, Check } from 'lucide-react';

export interface SelectOption {
    id: string;
    label: string;
    value: string;
    icon?: React.ReactNode;
}

interface CustomSelectProps {
    value: string;
    onChange: (value: string) => void;
    options: SelectOption[];
    placeholder?: string;
    disabled?: boolean;
    className?: string;
    filterable?: boolean;
    filterPlaceholder?: string;
}

export function CustomSelect({
    value,
    onChange,
    options,
    placeholder = "Select...",
    disabled = false,
    className = "",
    filterable = false,
    filterPlaceholder = "Filter...",
}: CustomSelectProps) {
    const [isOpen, setIsOpen] = useState(false);
    const [filter, setFilter] = useState("");
    const containerRef = useRef<HTMLDivElement>(null);

    const selectedOption = options.find(opt => opt.value === value);

    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
                setIsOpen(false);
            }
        };

        document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, []);

    // Reset filter when closing the menu so each open starts fresh.
    useEffect(() => {
        if (!isOpen && filter) {
            setFilter("");
        }
    }, [isOpen, filter]);

    const handleSelect = (optionValue: string) => {
        onChange(optionValue);
        setIsOpen(false);
    };

    const normalizedFilter = filter.toLowerCase();
    const filteredOptions = !filterable || !normalizedFilter
        ? options
        : options.filter((option) => {
              const label = option.label.toLowerCase();
              const valueStr = option.value.toLowerCase();
              return label.includes(normalizedFilter) || valueStr.includes(normalizedFilter);
          });

    return (
        <div ref={containerRef} className={`relative group ${className}`}>
            <button
                type="button"
                onClick={() => !disabled && setIsOpen(!isOpen)}
                disabled={disabled}
                className={`
                    w-full flex items-center justify-between gap-2 px-3 py-2.5 rounded-lg border text-sm font-medium transition-all
                    ${disabled
                        ? "opacity-50 cursor-not-allowed bg-[var(--color-bg-secondary)] border-[var(--color-border-secondary)] text-[var(--color-text-tertiary)]"
                        : isOpen
                            ? "bg-[var(--color-bg-primary)] border-[var(--color-accent-primary)] ring-1 ring-[var(--color-accent-primary)] text-[var(--color-text-primary)]"
                            : "bg-[var(--color-bg-tertiary)] border-[var(--color-border-secondary)] text-[var(--color-text-primary)] hover:bg-[var(--color-bg-secondary)]"
                    }
                `}
            >
                <div className="flex items-center gap-2 min-w-0">
                    {selectedOption?.icon && <span className="opacity-70">{selectedOption.icon}</span>}
                    <span className={`${!selectedOption ? "text-[var(--color-text-tertiary)]" : ""} truncate`}>
                        {selectedOption ? selectedOption.label : placeholder}
                    </span>
                </div>
                <ChevronDown size={16} className={`text-[var(--color-text-tertiary)] transition-transform ${isOpen ? "rotate-180" : ""}`} />
            </button>

            {/* Tooltip for full selected label (WebView-safe; avoids relying on native title tooltips) */}
            {!isOpen && selectedOption?.label && (
                <div className="absolute left-0 bottom-full mb-1 z-50 pointer-events-none opacity-0 group-hover:opacity-100 transition-opacity">
                    <div className="max-w-[420px] whitespace-normal break-words px-2 py-1 rounded border border-[var(--color-border-secondary)] bg-[var(--color-bg-elevated)] text-[var(--color-text-primary)] text-xs shadow-xl">
                        {selectedOption.label}
                    </div>
                </div>
            )}

            {isOpen && (
                <div className="absolute top-full left-0 right-0 mt-1 max-h-60 overflow-y-auto rounded-lg border border-[var(--color-border-primary)] bg-[var(--color-bg-elevated)] shadow-xl z-50 py-1 animate-in fade-in zoom-in-95 duration-100">
                    {filterable && options.length > 0 && (
                        <div className="px-2 pb-1">
                            <input
                                type="text"
                                value={filter}
                                onChange={(e) => setFilter(e.target.value)}
                                placeholder={filterPlaceholder}
                                className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded px-2 py-1 text-xs text-[var(--color-text-primary)]"
                            />
                        </div>
                    )}
                    {options.length > 0 ? (
                        filteredOptions.length > 0 ? (
                            filteredOptions.map((option) => (
                                <button
                                    key={option.id}
                                    onClick={() => handleSelect(option.value)}
                                    className={`
                                        w-full px-3 py-2 text-sm text-left flex items-center gap-2 transition-colors
${option.value === value
                                            ? "bg-[var(--color-bg-tertiary)] text-[var(--color-accent-primary)] font-semibold"
                                            : "text-[var(--color-text-primary)] hover:bg-[var(--color-hover-bg)]"
                                        }
                                    `}
                                >
                                    {option.icon && <span className="opacity-70 w-4 h-4 flex items-center justify-center">{option.icon}</span>}
                                    <span
                                        className="flex-1 whitespace-normal break-words"
                                        title={option.label}
                                    >
                                        {option.label}
                                    </span>
                                    {option.value === value && <Check size={14} className="flex-shrink-0" />}
                                </button>
                            ))
                        ) : (
                            <div className="px-3 py-2 text-xs text-[var(--color-text-tertiary)] text-center italic">
                                No matching options
                            </div>
                        )
                    ) : (
                        <div className="px-3 py-2 text-xs text-[var(--color-text-tertiary)] text-center italic">
                            No options available
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}
