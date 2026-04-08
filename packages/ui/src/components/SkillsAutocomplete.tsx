import { useRef, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { cn } from '../lib/cn';
import type { Skill } from '../hooks/useSkills';

interface SkillsAutocompleteProps {
  skills: Skill[];
  selectedIndex: number;
  onSelect: (skill: Skill) => void;
  onClose: () => void;
  visible: boolean;
  anchorRef?: React.RefObject<HTMLElement | null>;
}

export function SkillsAutocomplete({
  skills,
  selectedIndex,
  onSelect,
  onClose,
  visible,
  anchorRef,
}: SkillsAutocompleteProps) {
  const dropdownRef = useRef<HTMLDivElement>(null);
  const selectedRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ top: 0, left: 0, width: 0, placement: 'bottom' as 'bottom' | 'top' });

  // Calculate position based on anchor element with auto-placement
  useEffect(() => {
    if (!visible || !anchorRef?.current) return;

    const updatePosition = () => {
      const anchor = anchorRef.current;
      if (!anchor) return;
      
      const rect = anchor.getBoundingClientRect();
      const dropdownHeight = 240; // max-h-60 = 15rem = 240px
      const margin = 4;
      
      // Check if there's enough space below
      const spaceBelow = window.innerHeight - rect.bottom;
      const spaceAbove = rect.top;
      
      // Prefer bottom, but if not enough space and more space above, place on top
      const placement = spaceBelow < dropdownHeight && spaceAbove > spaceBelow ? 'top' : 'bottom';
      
      const top = placement === 'bottom' 
        ? rect.bottom + window.scrollY + margin
        : rect.top + window.scrollY - dropdownHeight - margin;
      
      setPosition({
        top,
        left: rect.left + window.scrollX,
        width: rect.width,
        placement,
      });
    };

    updatePosition();
    
    // Update on resize/scroll
    window.addEventListener('resize', updatePosition);
    window.addEventListener('scroll', updatePosition, true);
    
    return () => {
      window.removeEventListener('resize', updatePosition);
      window.removeEventListener('scroll', updatePosition, true);
    };
  }, [visible, anchorRef]);

  // Click outside to close
  useEffect(() => {
    if (!visible) return;
    
    const handleClickOutside = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        // Don't close if clicking on the anchor element
        if (anchorRef?.current && anchorRef.current.contains(e.target as Node)) {
          return;
        }
        onClose();
      }
    };

    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [visible, onClose, anchorRef]);

  // Scroll selected into view
  useEffect(() => {
    if (selectedRef.current) {
      selectedRef.current.scrollIntoView({ block: 'nearest' });
    }
  }, [selectedIndex]);

  if (!visible || skills.length === 0) return null;

  const dropdown = (
    <div
      ref={dropdownRef}
      className="fixed z-[9999] w-64 max-h-60 overflow-auto rounded-md border shadow-lg"
      style={{ 
        top: `${position.top}px`, 
        left: `${position.left}px`,
        minWidth: `${position.width}px`,
        backgroundColor: 'hsl(var(--popover))',
        borderColor: 'hsl(var(--border))',
        color: 'hsl(var(--popover-foreground))',
      }}
    >
      <div className="py-1" style={{ backgroundColor: 'hsl(var(--popover))' }}>
        <div 
          className="px-3 py-1.5 text-xs border-b"
          style={{ 
            color: 'hsl(var(--muted-foreground))',
            borderColor: 'hsl(var(--border))',
            backgroundColor: 'hsl(var(--popover))',
          }}
        >
          Skills ({skills.length})
        </div>
        {skills.map((skill, index) => (
          <div
            key={skill.id}
            ref={index === selectedIndex ? selectedRef : null}
            onClick={() => onSelect(skill)}
            className={cn(
              'px-3 py-2 cursor-pointer transition-colors',
              index === selectedIndex && 'text-accent-foreground'
            )}
            style={{
              backgroundColor: index === selectedIndex 
                ? 'hsl(var(--accent))' 
                : 'transparent',
            }}
            onMouseEnter={(e) => {
              if (index !== selectedIndex) {
                e.currentTarget.style.backgroundColor = 'hsl(var(--accent) / 0.5)';
              }
            }}
            onMouseLeave={(e) => {
              if (index !== selectedIndex) {
                e.currentTarget.style.backgroundColor = 'transparent';
              }
            }}
          >
            <div className="font-medium text-sm" style={{ color: 'hsl(var(--popover-foreground))' }}>
              {skill.name}
            </div>
            <div 
              className="text-xs truncate"
              style={{ 
                color: index === selectedIndex 
                  ? 'hsl(var(--accent-foreground) / 0.7)' 
                  : 'hsl(var(--muted-foreground))',
              }}
            >
              {skill.description}
            </div>
          </div>
        ))}
      </div>
    </div>
  );

  return createPortal(dropdown, document.body);
}
